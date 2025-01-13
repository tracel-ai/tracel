use std::path::{Path, PathBuf};
use std::process::Child;
use ipc_channel::ipc::{IpcOneShotServer, IpcReceiver, IpcSender};
use serde::{Deserialize, Serialize};
use tracel::heat::schemas::RegisteredHeatFunction;

#[derive(Serialize, Deserialize, Debug)]
pub enum ParentCliEvent {
    Input(super::app::CliParser),
    Sync,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ChildCliEvent {
    Output(Option<String>),
    SyncResponse(SyncInfo),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SyncInfo {
    pub current_dir: PathBuf,
    pub functions: Vec<RegisteredHeatFunction>,
}


pub trait CliProcess {
    fn send_event(&mut self, command: ParentCliEvent) -> anyhow::Result<()>;

    fn receive_event(&self) -> anyhow::Result<ChildCliEvent>;
}

pub struct IpcCliProcess {
    child_process: Child,
    cmd_sender: IpcSender<ParentCliEvent>,
    resp_receiver: IpcReceiver<ChildCliEvent>,
}

impl IpcCliProcess {
    /// Spawn a new child process with IPC channels
    pub fn new(bootstrapped_program: &Path) -> anyhow::Result<Self> {
        let (cmd_server, cmd_server_name) = IpcOneShotServer::<IpcSender<ParentCliEvent>>::new()?;
        let (resp_server, resp_server_name) = IpcOneShotServer::<IpcReceiver<ChildCliEvent>>::new()?;

        let child_process = std::process::Command::new(bootstrapped_program.as_os_str())
            .env("HEATCLI_REMOTE", "1")
            .env("HEATCLI_PARENT_CHANNEL_ID", &cmd_server_name)
            .env("HEATCLI_CHILD_CHANNEL_ID", &resp_server_name)
            .spawn()?;

        let (_cmd_receiver, cmd_sender) = cmd_server.accept()?;
        let (_resp_receiver, resp_receiver) = resp_server.accept()?;
        
        Ok(Self {
            child_process,
            cmd_sender,
            resp_receiver,
        })
    }

    pub fn wait(&mut self) -> anyhow::Result<()> {
        // return ok if child is already dead
        if self.child_process.try_wait()?.is_some() {
            return Ok(());
        }

        // wait for the child process to exit
        self.child_process.wait()?;
        Ok(())
    }

    pub fn kill(&mut self) -> anyhow::Result<()> {
        // return ok if child is already dead
        if self.child_process.try_wait()?.is_some() {
            return Ok(());
        }

        // kill the child process
        self.child_process.kill()?;
        Ok(())
    }
}

impl CliProcess for IpcCliProcess {
    /// Send a command to the child process
    fn send_event(&mut self, event: ParentCliEvent) -> anyhow::Result<()> {
        self.cmd_sender.send(event)?;
        Ok(())
    }

    /// Receive a response from the child process
    fn receive_event(&self) -> anyhow::Result<ChildCliEvent> {
        let event = self.resp_receiver.recv()?;
        Ok(event)
    }
}


