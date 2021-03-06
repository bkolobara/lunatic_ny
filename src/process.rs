use std::{collections::HashSet, future::Future, hash::Hash};

use anyhow::Result;
use log::debug;
use tokio::{
    sync::{
        broadcast::{Receiver, Sender},
        mpsc::{UnboundedReceiver, UnboundedSender},
    },
    task::JoinHandle,
};
use uuid::Uuid;
use wasmtime::Val;

use crate::message::Message;

#[derive(Debug)]
pub enum Signal {
    // When received process should stop.
    Kill,
    // Change behaviour of what happens if a linked process dies.
    DieWhenLinkDies(bool),
    // Sent from a process that wants to be linked.
    Link(ProcessHandle),
    // Sent to linked processes when a process dies because of a trap.
    LinkNotifyTrap,
    // Sent to linked processes when a process dies because of a kill signal.
    LinkNotifyKill,
}

/// The reason of a process finishing
pub enum Finished<T> {
    /// The Wasm function finished or trapped
    Wasm(T),
    /// The process was terminated by an external signal
    Signal(Signal),
}

/// The only way of communicating with processes is through a `ProcessHandle`.
///
/// Lunatic processes can be crated from a Wasm module & exported function name (or table index).
/// They are created inside the `Environment::spawn` method, and once spawned they will be running
/// in the background and can't be observed directly.
#[derive(Debug)]
pub struct ProcessHandle {
    id: Uuid,
    signal_sender: UnboundedSender<Signal>,
    message_sender: UnboundedSender<Message>,
    trapped_sender: Sender<bool>,
    trapped: Receiver<bool>,
}

impl Clone for ProcessHandle {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            signal_sender: self.signal_sender.clone(),
            message_sender: self.message_sender.clone(),
            trapped_sender: self.trapped_sender.clone(),
            trapped: self.trapped_sender.subscribe(),
        }
    }
}

impl PartialEq for ProcessHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for ProcessHandle {}

impl Hash for ProcessHandle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl ProcessHandle {
    /// Create a new ProcessHandle
    pub fn new(
        id: Uuid,
        signal_sender: UnboundedSender<Signal>,
        message_sender: UnboundedSender<Message>,
        trapped_sender: Sender<bool>,
    ) -> Self {
        Self {
            id,
            signal_sender,
            message_sender,
            trapped_sender: trapped_sender.clone(),
            trapped: trapped_sender.subscribe(),
        }
    }

    /// Send message to process
    pub fn send_message(&self, message: Message) -> Result<()> {
        Ok(self.message_sender.send(message)?)
    }

    /// Send signal to process
    pub fn send_signal(&self, signal: Signal) -> Result<()> {
        Ok(self.signal_sender.send(signal)?)
    }

    /// Wait on process to finish and return:
    /// * false if the process finished normally
    /// * true if the process trapped or received a Signal::Kill
    pub async fn join(&mut self) -> bool {
        self.trapped
            .recv()
            .await
            .expect("a process holds a sender and must exist at this time")
    }
}

// Turns a Future into a process, enabling signals (e.g. kill).
pub(crate) fn new<F>(
    fut: F,
    message_sender: UnboundedSender<Message>,
    trapped_sender: Sender<bool>,
    mut signal_mailbox: UnboundedReceiver<Signal>,
) -> JoinHandle<()>
where
    F: Future<Output = Result<Box<[Val]>>> + Send + 'static,
{
    let process = async move {
        tokio::pin!(fut);

        // Defines what happens if one of the linked processes dies.
        let mut die_when_link_dies = true;
        // Process linked to this one
        let mut links = HashSet::new();
        let mut disable_signals = false;
        let result = loop {
            tokio::select! {
                biased;
                // Handle signals first
                signal = signal_mailbox.recv(), if !disable_signals => {
                    match signal {
                        Some(Signal::DieWhenLinkDies(value)) => die_when_link_dies = value,
                        // Put process into list of linked processes
                        Some(Signal::Link(proc)) => { links.insert(proc); },
                        // Exit loop and don't poll anymore the future if Signal::Kill received.
                        Some(Signal::Kill) => break Finished::Signal(Signal::Kill),
                        // Depending if `die_when_link_dies` is set, process will die or turn the
                        // signal into a message
                        Some(Signal::LinkNotifyTrap) | Some(Signal::LinkNotifyKill) => {
                            if die_when_link_dies {
                                // Even this was not a **kill** signal it has the same effect on
                                // this process and should be propagated as such.
                                break Finished::Signal(Signal::Kill)
                            } else {
                                message_sender.send(Message::Signal)
                                .expect("message is sent to ourself and receiver must exist");
                            }
                        },
                        // Can't receive anymore signals, disable this `select!` branch
                        None => disable_signals = true
                    }
                }
                // Run process
                output = &mut fut => { break Finished::Wasm(output); }
            }
        };
        match result {
            Finished::Wasm(Result::Err(err)) => {
                debug!("Process failed: {}", err);
                // Notify all links that we finished with a trap
                links.iter().for_each(|proc| {
                    let _ = proc.send_signal(Signal::LinkNotifyTrap);
                });
                // Notify process handles that we finished with a trap
                let _ = trapped_sender.send(true);
            }
            Finished::Signal(Signal::Kill) => {
                debug!("Process was killed");
                // Notify all links that we finished because of a kill signal
                links.iter().for_each(|proc| {
                    let _ = proc.send_signal(Signal::LinkNotifyKill);
                });
                // Notify process handles that we finished with a trap
                let _ = trapped_sender.send(true);
            }
            _ => {
                let _ = trapped_sender.send(false);
            }
        }
    };

    // Spawn a background process
    tokio::spawn(process)
}
