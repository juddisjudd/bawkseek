use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use rodio::{Decoder, DeviceSinkBuilder, Player};

const TICK: Duration = Duration::from_millis(200);

pub enum AudioCommand {
    Load(PathBuf),
    Play,
    Pause,
    Seek(Duration),
    Volume(f32),
    Stop,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AudioEvent {
    Position(Duration),
    Ended,
    Failed(String),
}

/// The sound device lives on its own thread, because the output stream must stay where it was opened.
pub struct Audio {
    commands: Sender<AudioCommand>,
    events: Receiver<AudioEvent>,
}

impl Audio {
    pub fn spawn() -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(command_rx, event_tx))
            .expect("spawn audio thread");
        Self { commands, events }
    }

    pub fn send(&self, command: AudioCommand) {
        let _ = self.commands.send(command);
    }

    pub fn try_event(&self) -> Option<AudioEvent> {
        self.events.try_recv().ok()
    }
}

fn open(path: &Path) -> Result<Decoder<BufReader<File>>, String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let len = file.metadata().map_err(|err| err.to_string())?.len();
    let hint = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    Decoder::builder()
        .with_data(BufReader::new(file))
        .with_byte_len(len)
        .with_hint(&hint)
        .with_seekable(true)
        .build()
        .map_err(|err| format!("cannot play this file: {err}"))
}

fn run(commands: Receiver<AudioCommand>, events: Sender<AudioEvent>) {
    let mut device = match DeviceSinkBuilder::open_default_sink() {
        Ok(device) => device,
        Err(err) => {
            let message = format!("no sound device: {err}");
            while let Ok(command) = commands.recv() {
                if matches!(command, AudioCommand::Load(_)) {
                    let _ = events.send(AudioEvent::Failed(message.clone()));
                }
            }
            return;
        }
    };
    device.log_on_drop(false);
    let player = Player::connect_new(device.mixer());
    let mut loaded = false;
    loop {
        match commands.recv_timeout(TICK) {
            Ok(AudioCommand::Load(path)) => {
                player.clear();
                loaded = false;
                match open(&path) {
                    Ok(source) => {
                        player.append(source);
                        player.play();
                        loaded = true;
                    }
                    Err(err) => {
                        let _ = events.send(AudioEvent::Failed(err));
                    }
                }
            }
            Ok(AudioCommand::Play) => player.play(),
            Ok(AudioCommand::Pause) => player.pause(),
            Ok(AudioCommand::Seek(position)) => {
                let _ = player.try_seek(position);
            }
            Ok(AudioCommand::Volume(volume)) => player.set_volume(volume.clamp(0.0, 1.0)),
            Ok(AudioCommand::Stop) => {
                player.clear();
                loaded = false;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if !loaded {
            continue;
        }
        if player.empty() {
            loaded = false;
            let _ = events.send(AudioEvent::Ended);
        } else if !player.is_paused() {
            let _ = events.send(AudioEvent::Position(player.get_pos()));
        }
    }
}
