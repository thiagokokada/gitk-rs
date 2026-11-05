//! Optional instrumentation. Rendering only sends fixed-size records to a bounded
//! queue; stderr output and sample aggregation happen on a separate thread.
use std::{
    collections::VecDeque,
    io::{self, Write},
    sync::mpsc::{self, SyncSender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub enum Phase {
    PreferencesLoaded,
    WorkersStarted,
    ApplicationCreated,
    ComponentsInitialized,
    WindowOpening,
    WindowCreated,
    ViewCreated,
    WindowPainted,
    HistoryPainted,
    SelectionPainted,
    DiffPainted,
}

impl Phase {
    const COUNT: usize = Self::DiffPainted as usize + 1;
}

enum Event {
    Phase(Phase, Duration),
    Frame(Duration),
    Stop,
}

#[derive(Clone)]
pub struct Timing {
    origin: Instant,
    sender: Option<SyncSender<Event>>,
}

pub struct TimingGuard {
    timing: Timing,
    worker: Option<JoinHandle<()>>,
}

impl TimingGuard {
    pub fn start(enabled: bool, origin: Instant) -> Self {
        let (sender, worker) = if enabled {
            let (sender, receiver) = mpsc::sync_channel(64);
            let worker = thread::spawn(move || {
                let mut samples = VecDeque::with_capacity(512);
                let mut first = [true; Phase::COUNT];
                let mut output = io::BufWriter::new(io::stderr());
                while let Ok(event) = receiver.recv() {
                    match event {
                        Event::Phase(phase, elapsed) => {
                            if first[phase as usize] {
                                first[phase as usize] = false;
                                let _ = writeln!(
                                    output,
                                    "gitk-rs timing phase={phase:?} elapsed_us={}",
                                    elapsed.as_micros()
                                );
                                let _ = output.flush();
                            }
                        }
                        Event::Frame(duration) => {
                            if samples.len() == 512 {
                                samples.pop_front();
                            }
                            samples.push_back(duration.as_micros());
                        }
                        Event::Stop => break,
                    }
                }
                if !samples.is_empty() {
                    let mut values: Vec<_> = samples.into_iter().collect();
                    values.sort_unstable();
                    let p95 = values[(values.len() * 95).div_ceil(100) - 1];
                    let _ = writeln!(
                        output,
                        "gitk-rs timing cpu_render_to_paint_samples={} p95_us={p95}",
                        values.len()
                    );
                }
                let _ = output.flush();
            });
            (Some(sender), Some(worker))
        } else {
            (None, None)
        };
        Self {
            timing: Timing { origin, sender },
            worker,
        }
    }

    pub fn timing(&self) -> Timing {
        self.timing.clone()
    }
}

impl Drop for TimingGuard {
    fn drop(&mut self) {
        if let Some(sender) = &self.timing.sender {
            // Joining is deliberately outside the GUI event loop.
            let _ = sender.send(Event::Stop);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Timing {
    pub fn enabled(&self) -> bool {
        self.sender.is_some()
    }

    pub fn mark(&self, phase: Phase) {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(Event::Phase(phase, self.origin.elapsed()));
        }
    }

    pub fn frame(&self, render_started: Instant) {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(Event::Frame(render_started.elapsed()));
        }
    }
}
