//! Glue: candidate → job → render → compose → narrate → archive row.

use std::path::PathBuf;

use rti_archive::rows::MediaRow;
use rti_core::ContentHash;
use rti_research::Session;

use crate::compare::compare;
use crate::compose::{write_ppm, Encoder};
use crate::detect::{scan, Candidate};
use crate::job::MediaJob;
use crate::narration::narrate;
use crate::render::{overview_frame, render_frames, Scene};

pub use rti_core::config::MediaConfig;

pub fn media_dir(s: &Session) -> PathBuf {
    s.cfg.data_dir.join("media")
}

/// Produce the video for a candidate and record it for review.
pub fn produce(s: &Session, cfg: &MediaConfig, c: &Candidate) -> anyhow::Result<MediaRow> {
    let track = s.track(&c.track_name)?;
    let new = s
        .archive
        .trajectory(&ContentHash(c.new_trajectory.clone()))?
        .ok_or_else(|| anyhow::anyhow!("trajectory missing"))?;
    let old = match &c.old_trajectory {
        Some(h) => s.archive.trajectory(&ContentHash(h.clone()))?,
        None => None,
    };
    anyhow::ensure!(
        !new.states.is_empty(),
        "new trajectory has no telemetry (states)"
    );
    let cmp = old
        .as_ref()
        .filter(|o| !o.states.is_empty())
        .map(|o| compare(&new, o, track.length(), 8));
    let mut job = MediaJob::short(
        &c.track_name,
        &c.new_trajectory,
        c.old_trajectory.as_deref(),
    );
    job.width = cfg.width;
    job.height = cfg.height;
    job.fps = cfg.fps;
    job.map_hash = s.archive.map_by_track(&c.track_name)?.map(|m| m.hash);
    if let Some(r) = &c.reference_label {
        job.old_label = if old.is_some() {
            "previous best".into()
        } else {
            r.clone()
        };
    }
    let dir = media_dir(s);
    std::fs::create_dir_all(&dir)?;
    let out = dir.join(format!("{}.mp4", job.id));
    let scene = Scene {
        job: &job,
        track: &track,
        new: &new,
        old: old.as_ref().filter(|o| !o.states.is_empty()),
        comparison: cmp.as_ref(),
        reference_label: c.reference_label.clone(),
    };
    let timer = rti_core::ledger::CostTimer::start();
    let mut enc = Encoder::start(&out, job.width, job.height, job.fps, cfg.crf)?;
    let frames = render_frames(&scene, |f| enc.push(f))?;
    let path = enc.finish()?;
    let thumb = overview_frame(&track, &new, old.as_ref(), 640, 640);
    let thumb_path = dir.join(format!("{}.ppm", job.id));
    write_ppm(&thumb_path, &thumb)?;
    let narration = narrate(s, c, cmp.as_ref());
    let bytes = std::fs::read(&path)?;
    let video_hash = s.archive.cas.put_bytes(&bytes)?;
    let cost = timer.finish(0);
    let row = MediaRow {
        id: job.id.clone(),
        kind: job.kind.clone(),
        track_name: c.track_name.clone(),
        new_trajectory: c.new_trajectory.clone(),
        old_trajectory: c.old_trajectory.clone(),
        score: c.interest.score,
        interest: Some(serde_json::to_value(&c.interest)?),
        job: Some(serde_json::to_value(&job)?),
        comparison: cmp
            .as_ref()
            .map(|c| serde_json::to_value(c).unwrap_or_default()),
        video_path: Some(path.display().to_string()),
        video_hash: Some(video_hash.0.clone()),
        thumbnail_path: Some(thumb_path.display().to_string()),
        title: narration.title.clone(),
        description: narration.description.clone(),
        explanation: narration.explanation.clone(),
        tags: narration.tags.clone(),
        status: "review".into(),
        youtube_id: None,
        verified: c.verified,
        frames: frames as i64,
        created_at: String::new(),
        updated_at: String::new(),
    };
    s.archive.insert_media(&row)?;
    s.archive.ledger_add(
        "media",
        &row.id,
        &cost,
        &format!("render {} frames for {}", frames, c.track_name),
    )?;
    tracing::info!(id = %row.id, frames, "media produced: {}", row.title);
    Ok(row)
}

/// Scan for candidates above threshold and produce them. Returns new rows.
pub fn scan_and_produce(s: &Session, cfg: &MediaConfig) -> anyhow::Result<Vec<MediaRow>> {
    if !cfg.enabled {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for c in scan(s, &cfg.weights, cfg.require_verified)? {
        if c.interest.score < cfg.weights.threshold {
            continue;
        }
        match produce(s, cfg, &c) {
            Ok(r) => out.push(r),
            Err(e) => tracing::warn!(track = %c.track_name, error = %e, "media production failed"),
        }
    }
    Ok(out)
}

/// Upload a reviewed video (private by default) and optionally publish.
pub fn upload(s: &Session, cfg: &MediaConfig, id: &str, publish: bool) -> anyhow::Result<String> {
    let row = s
        .archive
        .media_by_id(id)?
        .ok_or_else(|| anyhow::anyhow!("no media {id}"))?;
    let yt = crate::publish::YoutubeClient::new(&cfg.youtube);
    anyhow::ensure!(
        yt.is_configured(),
        "YouTube is not configured (set {}, {}, {})",
        cfg.youtube.client_id_env,
        cfg.youtube.client_secret_env,
        cfg.youtube.refresh_token_env
    );
    let path = row
        .video_path
        .clone()
        .ok_or_else(|| anyhow::anyhow!("media {id} has no video file"))?;
    let vid = match &row.youtube_id {
        Some(v) => v.clone(),
        None => {
            let v = yt.upload(
                std::path::Path::new(&path),
                &row.title,
                &row.description,
                &row.tags,
                &cfg.youtube.upload_privacy,
            )?;
            s.archive.update_media(id, "uploaded", Some(&v))?;
            v
        }
    };
    if publish {
        yt.set_privacy(&vid, "public")?;
        s.archive.update_media(id, "published", Some(&vid))?;
    }
    Ok(vid)
}
