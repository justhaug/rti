//! Title, description and explanation for a video, from the archive's
//! ancestry (experiment → hypothesis → method) and the comparison. Uses the
//! LLM when configured, with a deterministic template fallback.

use rti_llm::{ChatOptions, Message};
use rti_research::Session;
use serde::{Deserialize, Serialize};

use crate::compare::Comparison;
use crate::detect::Candidate;
use crate::render::fmt_time;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Narration {
    pub title: String,
    pub description: String,
    /// One or two sentences for the "why it works" card / pinned comment.
    pub explanation: String,
    pub tags: Vec<String>,
}

/// Lineage of a trajectory: the chain of experiments that produced it.
pub fn ancestry(s: &Session, traj_hash: &str) -> anyhow::Result<Vec<String>> {
    let mut lines = vec![];
    let mut cur = Some(traj_hash.to_string());
    let mut guard = 0;
    while let Some(h) = cur {
        guard += 1;
        if guard > 12 {
            break;
        }
        let Some(row) = s
            .archive
            .trajectory_row(&rti_core::ContentHash(h.clone()))?
        else {
            break;
        };
        let exp = match &row.experiment_id {
            Some(id) => s.archive.experiment(id)?,
            None => None,
        };
        let hyp = match exp.as_ref().and_then(|e| e.hypothesis_id.clone()) {
            Some(hid) => s
                .archive
                .hypotheses(None, 1000)?
                .into_iter()
                .find(|x| x.id == hid)
                .map(|x| x.text),
            None => None,
        };
        lines.push(format!(
            "{} ({}): {} ms via {}{}",
            &h[..12.min(h.len())],
            row.world,
            row.time_ms,
            row.method.clone().unwrap_or_default(),
            hyp.map(|t| format!(" — hypothesis: {t}"))
                .unwrap_or_default()
        ));
        cur = row.parent.clone();
    }
    Ok(lines)
}

pub fn template(c: &Candidate, cmp: Option<&Comparison>, lineage: &[String]) -> Narration {
    let delta = c.old_time_ms.map(|o| c.new_time_ms as i32 - o as i32);
    let title = match delta {
        Some(d) if d < 0 => format!(
            "RTI beats its best on {} by {:.3}s ({})",
            c.track_name,
            -d as f32 / 1000.0,
            fmt_time(c.new_time_ms)
        ),
        _ => format!("RTI drives {} in {}", c.track_name, fmt_time(c.new_time_ms)),
    };
    let mut description = format!("Recursive Trackmania Intelligence: an autonomous research system that searches a replicated physics simulator and verifies results in the oracle.\n\nTrack: {}\nNew time: {}\n", c.track_name, fmt_time(c.new_time_ms));
    if let Some(o) = c.old_time_ms {
        description.push_str(&format!("Previous best: {}\n", fmt_time(o)));
    }
    if let Some(r) = &c.reference_label {
        description.push_str(&format!("Reference: {r}\n"));
    }
    if let Some(cmp) = cmp {
        description.push_str("\nWhere the time came from:\n");
        for b in &cmp.bullets {
            description.push_str(&format!("- {b}\n"));
        }
    }
    if !lineage.is_empty() {
        description.push_str("\nLineage:\n");
        for l in lineage {
            description.push_str(&format!("- {l}\n"));
        }
    }
    description.push_str(&format!(
        "\nInterestingness {:.2}: {}\n",
        c.interest.score,
        c.interest.reasons.join(", ")
    ));
    let explanation = cmp.and_then(|c| c.biggest_gain.as_ref()).map(|g| format!("Most of the gain ({} ms) comes from the section at {:.0}-{:.0} m, exiting {:+.1} km/h faster.", -g.section_delta_ms, g.from_m, g.to_m, (g.new_exit_speed - g.old_exit_speed) * 3.6)).unwrap_or_default();
    Narration {
        title,
        description,
        explanation,
        tags: vec![
            "trackmania".into(),
            "RTI".into(),
            "AI".into(),
            c.track_name.clone(),
        ],
    }
}

pub fn narrate(s: &Session, c: &Candidate, cmp: Option<&Comparison>) -> Narration {
    let lineage = ancestry(s, &c.new_trajectory).unwrap_or_default();
    let base = template(c, cmp, &lineage);
    if !s.llm.is_configured() {
        return base;
    }
    let system = "You write titles and descriptions for short videos published by RTI, an autonomous Trackmania research system. Be factual, specific and short; never invent numbers that are not in the input. Reply ONLY with JSON {\"title\": <≤80 chars>, \"description\": <≤600 chars, plain text with line breaks>, \"explanation\": <one or two sentences on why the new line is faster, grounded in the comparison>, \"tags\": [<≤6 short tags>]}.";
    let user = format!(
        "Facts:\n{}\n\nComparison:\n{}\n\nDraft (improve it):\n{}",
        serde_json::to_string_pretty(c).unwrap_or_default(),
        cmp.map(|c| serde_json::to_string_pretty(c).unwrap_or_default())
            .unwrap_or_default(),
        serde_json::to_string_pretty(&base).unwrap_or_default()
    );
    match s.llm.chat_json::<Narration>(
        "researcher",
        &[Message::system(system), Message::user(user)],
        &ChatOptions {
            temperature: Some(0.5),
            ..Default::default()
        },
    ) {
        Ok((mut n, _)) => {
            if n.title.trim().is_empty() {
                n.title = base.title;
            }
            if n.description.trim().is_empty() {
                n.description = base.description;
            }
            n
        }
        Err(e) => {
            tracing::warn!(error = %e, "narration LLM failed; using template");
            base
        }
    }
}
