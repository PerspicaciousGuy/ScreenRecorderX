use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn system() -> Result<Self> {
        let dirs = directories::ProjectDirs::from("org", "Clearcast", "Clearcast")
            .context("Cannot locate application data directory")?;
        Self::new(dirs.data_local_dir().to_owned())
    }
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join("pending"))?;
        Ok(Self { root })
    }
    pub fn lock(&self) -> Result<File> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("instance.lock"))?;
        lock.try_lock_exclusive()
            .context("Clearcast is already running; close the other instance first")?;
        Ok(lock)
    }
    pub fn load_settings(&self) -> Result<crate::config::Settings> {
        let path = self.root.join("settings.json");
        if !path.exists() {
            return Ok(Default::default());
        }
        let settings: crate::config::Settings = serde_json::from_slice(&fs::read(path)?)
            .context("Settings are unreadable; rename settings.json to restore defaults")?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn save_settings(&self, settings: &crate::config::Settings) -> Result<()> {
        settings.validate()?;
        atomic_json(&self.root.join("settings.json"), settings)
    }
    pub fn begin(&self, output: &Path) -> Result<PendingRecording> {
        fs::create_dir_all(output).context("Cannot create output directory")?;
        let output = output.canonicalize()?;
        ensure!(
            fs2::available_space(&output)? >= 256 * 1024 * 1024,
            "Less than 256 MiB free in output directory"
        );
        let id = Uuid::new_v4();
        let base = format!(
            "Clearcast-{}-{}",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            &id.to_string()[..8]
        );
        let pending = PendingRecording {
            id,
            started: chrono::Utc::now().to_rfc3339(),
            mkv: output.join(format!(".{base}.mkv")),
            mp4: output.join(format!("{base}.mp4")),
        };
        atomic_json(&self.journal(id), &pending)?;
        // Reserve the capture path, refusing symlinks and existing recordings.
        if let Err(error) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending.mkv)
        {
            fs::remove_file(self.journal(id))?;
            return Err(error.into());
        }
        Ok(pending)
    }
    pub fn journal(&self, id: Uuid) -> PathBuf {
        self.root.join("pending").join(format!("{id}.json"))
    }
    pub fn pending(&self) -> Result<Vec<PendingRecording>> {
        let mut recordings = Vec::new();
        for entry in fs::read_dir(self.root.join("pending"))? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            // One malformed journal must not hide other recoverable recordings.
            match fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<PendingRecording>(&b).ok())
            {
                Some(recording) => recordings.push(recording),
                None => eprintln!("Unusable recovery journal: {}", path.display()),
            }
        }
        recordings.sort_by(|a, b| a.started.cmp(&b.started));
        Ok(recordings)
    }
    pub fn forget_empty(&self, recording: &PendingRecording) -> Result<()> {
        if recording.mkv.metadata().map(|m| m.len()).unwrap_or(0) == 0 && !recording.mp4.exists() {
            if recording.mkv.exists() {
                fs::remove_file(&recording.mkv)?;
            }
            fs::remove_file(self.journal(recording.id))?;
        }
        Ok(())
    }
    pub fn finalize(&self, recording: &PendingRecording) -> Result<PathBuf> {
        ensure!(
            recording.mkv.is_file(),
            "Recovery source is missing: {}",
            recording.mkv.display()
        );
        ensure!(
            !recording.mp4.exists(),
            "Destination already exists; source retained: {}",
            recording.mp4.display()
        );
        let temp = tempfile::Builder::new()
            .prefix(".clearcast-")
            .suffix(".partial")
            .tempfile_in(
                recording
                    .mp4
                    .parent()
                    .context("Invalid recording destination")?,
            )?
            .into_temp_path();
        fs::remove_file(&temp)?; // FFmpeg must create it with -n (no overwrite).
        let result = (|| -> Result<()> {
            let ffmpeg = ffmpeg_path();
            let log = tempfile::tempfile()?;
            let mut command = Command::new(&ffmpeg);
            command
                .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-n", "-i"])
                .arg(&recording.mkv)
                .args([
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a:0?",
                    "-c",
                    "copy",
                    "-movflags",
                    "+faststart",
                    "-f",
                    "mp4",
                ])
                .arg(&temp)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(log.try_clone()?);
            hide_console(&mut command);
            let status = command.status().with_context(|| {
                format!(
                    "Cannot launch {}. Install FFmpeg or place it beside Clearcast",
                    ffmpeg.display()
                )
            })?;
            let mut log = log;
            let size = log.metadata()?.len();
            log.seek(SeekFrom::Start(size.saturating_sub(16384)))?;
            let mut tail = String::new();
            log.read_to_string(&mut tail)?;
            ensure!(
                status.success(),
                "MP4 conversion failed; MKV retained. {tail}"
            );
            ensure!(temp.metadata()?.len() > 0, "FFmpeg produced an empty file");
            // Windows FlushFileBuffers requires a handle opened for writing.
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&temp)?
                .sync_all()
                .context("Cannot sync finalized MP4; MKV retained")?;
            // Persist without overwriting a file created while FFmpeg was running.
            // tempfile uses the platform's no-clobber publication primitive.
            tempfile::TempPath::try_from_path(&temp)?
                .persist_noclobber(&recording.mp4)
                .context("Cannot publish MP4 without overwriting an existing file; MKV retained")?;
            sync_directory(recording.mp4.parent().context("Invalid MP4 path")?)?;
            fs::remove_file(&recording.mkv)?;
            fs::remove_file(self.journal(recording.id))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result?;
        Ok(recording.mp4.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingRecording {
    pub id: Uuid,
    pub started: String,
    pub mkv: PathBuf,
    pub mp4: PathBuf,
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().context("Invalid settings path")?)?;
    temp.write_all(&serde_json::to_vec_pretty(value)?)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    sync_directory(path.parent().context("Invalid settings path")?)?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn ffmpeg_path() -> PathBuf {
    let filename = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if let Ok(executable) = std::env::current_exe() {
        let sibling = executable.with_file_name(filename);
        if sibling.is_file() {
            return sibling;
        }
        let bundled = executable
            .parent()
            .unwrap_or(Path::new("."))
            .join("bin")
            .join(filename);
        if bundled.is_file() {
            return bundled;
        }
    }
    filename.into()
}

pub fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    let _ = command;
}
