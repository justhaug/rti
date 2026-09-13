//! FFmpeg composition: streams RGB frames into an H.264 MP4 (9:16 Shorts
//! friendly). ffmpeg is located via `$RTI_FFMPEG`, then `PATH`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::render::Frame;

pub fn ffmpeg_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RTI_FFMPEG") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join("ffmpeg");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Encoder that accepts frames and writes `out`.
pub struct Encoder {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    pub frames: usize,
    pub out: PathBuf,
}

impl Encoder {
    pub fn start(out: &Path, w: u32, h: u32, fps: u32, crf: u32) -> anyhow::Result<Encoder> {
        let ffmpeg = ffmpeg_path()
            .ok_or_else(|| anyhow::anyhow!("ffmpeg not found (install it or set RTI_FFMPEG)"))?;
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut child = Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
            ])
            .args(["-s", &format!("{w}x{h}"), "-r", &fps.to_string(), "-i", "-"])
            .args([
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                &crf.to_string(),
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ])
            .arg(out)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        Ok(Encoder {
            child,
            stdin,
            frames: 0,
            out: out.to_path_buf(),
        })
    }

    pub fn push(&mut self, f: &Frame) -> anyhow::Result<()> {
        if let Some(s) = self.stdin.as_mut() {
            s.write_all(&f.px)?;
        }
        self.frames += 1;
        Ok(())
    }

    pub fn finish(mut self) -> anyhow::Result<PathBuf> {
        drop(self.stdin.take());
        let out = self.child.wait_with_output()?;
        if !out.status.success() {
            anyhow::bail!("ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(self.out)
    }
}

/// Write a single frame as a PNG-free PPM (for thumbnails without deps).
pub fn write_ppm(path: &Path, f: &Frame) -> anyhow::Result<()> {
    let mut data = format!("P6\n{} {}\n255\n", f.w, f.h).into_bytes();
    data.extend_from_slice(&f.px);
    std::fs::write(path, data)?;
    Ok(())
}

/// Duration in seconds via ffprobe when available.
pub fn probe_duration(path: &Path) -> Option<f64> {
    let ffprobe = ffmpeg_path()?.with_file_name("ffprobe");
    let out = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}
