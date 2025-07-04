use std::{sync::mpsc, thread::JoinHandle};

use crate::websocket::WebSocketClient;

use super::{TempLogStore, WsMessage};

pub trait ExperimentThread<R> {
    fn run(self) -> R;
}

#[derive(Debug)]
pub struct WSThreadResult {
    pub logs: TempLogStore,
}

struct ExperimentWSThread {
    ws_client: WebSocketClient,
    receiver: mpsc::Receiver<WsMessage>,
    in_memory_logs: TempLogStore,
    iteration_count: usize,
}

impl ExperimentWSThread {
    pub fn new(
        ws_client: WebSocketClient,
        receiver: mpsc::Receiver<WsMessage>,
        in_memory_logs: TempLogStore,
    ) -> Self {
        Self {
            ws_client,
            receiver,
            in_memory_logs,
            iteration_count: 0,
        }
    }
}

impl ExperimentThread<WSThreadResult> for ExperimentWSThread {
    fn run(mut self) -> WSThreadResult {
        let mut logs = self.in_memory_logs;

        loop {
            let res = self.receiver.recv();
            if res.is_err() {
                break;
            }
            match res.unwrap() {
                WsMessage::MetricLog {
                    name,
                    epoch,
                    iteration: _iteration,
                    value,
                    group,
                } => {
                    self.iteration_count += 1;
                    self.ws_client
                        .send(WsMessage::MetricLog {
                            name,
                            epoch,
                            iteration: self.iteration_count,
                            value,
                            group,
                        })
                        .unwrap();
                }
                WsMessage::Log(log) => {
                    logs.push(log.clone()).unwrap();
                    self.ws_client.send(WsMessage::Log(log)).unwrap();
                }
                WsMessage::Error(err) => {
                    self.ws_client.send(WsMessage::Error(err)).unwrap();
                }
                WsMessage::Close => {
                    break;
                }
            }
        }
        self.ws_client.close().unwrap();

        WSThreadResult { logs }
    }
}

#[derive(Debug)]
pub struct ExperimentWSHandler {
    sender: mpsc::Sender<WsMessage>,
    handle: JoinHandle<WSThreadResult>,
}

impl ExperimentWSHandler {
    pub fn new(ws_client: WebSocketClient, log_store: TempLogStore) -> Self {
        let (sender, receiver) = mpsc::channel();

        let thread = ExperimentWSThread::new(ws_client, receiver, log_store);
        let handle = std::thread::spawn(move || thread.run());

        Self { sender, handle }
    }

    pub fn get_sender(&self) -> mpsc::Sender<WsMessage> {
        self.sender.clone()
    }

    pub fn join(self) -> WSThreadResult {
        self.sender.send(WsMessage::Close).unwrap();
        self.handle.join().unwrap()
    }
}
