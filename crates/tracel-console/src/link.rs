//! The connection actor: owns the console session and runs every call against it on the
//! backend's executor.

use std::sync::Arc;

use tracel_client::console::Client;
use tracel_task::{Aborted, Spawn, Task};

enum Command {
    Call(Box<dyn FnOnce(&Client) + Send>),
}

/// A handle to the connection actor. Clones share the session.
#[derive(Clone)]
pub struct Link {
    tx: async_channel::Sender<Command>,
}

impl Link {
    pub fn start(client: Client, spawn: Arc<dyn Spawn>) -> Self {
        let (tx, rx) = async_channel::bounded(64);
        spawn.spawn(Box::pin(run(client, rx, Arc::clone(&spawn))));
        Self { tx }
    }

    /// Runs `call` against the session and resolves to its result.
    pub async fn call<T, E, F>(&self, call: F) -> Result<T, E>
    where
        F: FnOnce(&Client) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + From<Aborted> + 'static,
    {
        let (reply, task) = Task::channel();
        let job = Box::new(move |client: &Client| reply.send(call(client)));
        self.tx
            .send(Command::Call(job))
            .await
            .map_err(|_| Aborted)?;
        task.await
    }
}

/// Dispatches, never executes: a call runs off the loop so the session stays responsive.
///
/// Until `tracel-client` is asynchronous every call blocks, so it goes to the executor's blocking
/// lane rather than its scheduler.
async fn run(client: Client, rx: async_channel::Receiver<Command>, spawn: Arc<dyn Spawn>) {
    while let Ok(command) = rx.recv().await {
        match command {
            Command::Call(call) => {
                let client = client.clone();
                spawn.spawn_blocking(Box::new(move || call(&client)));
            }
        }
    }
}
