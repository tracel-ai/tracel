use std::{sync::mpsc, thread::JoinHandle};

use crate::websocket::WebSocketClient;

use super::{TempLogStore, ExperimentMessage};

#[derive(Debug)]
pub struct WSThreadResult {
    pub logs: TempLogStore,
}

struct ExperimentWSThread {
    ws_client: WebSocketClient,
    receiver: mpsc::Receiver<ExperimentMessage>,
    in_memory_logs: TempLogStore,
    iteration_count: usize,
}

impl ExperimentWSThread {
    pub fn new(
        ws_client: WebSocketClient,
        receiver: mpsc::Receiver<ExperimentMessage>,
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

impl ExperimentWSThread {
    fn run(mut self) -> WSThreadResult {
        let mut logs = self.in_memory_logs;

        loop {
            let res = self.receiver.recv();
            if res.is_err() {
                break;
            }
            match res.unwrap() {
                ExperimentMessage::MetricLog {
                    name,
                    epoch,
                    iteration: _iteration,
                    value,
                    group,
                } => {
                    self.iteration_count += 1;
                    self.ws_client
                        .send(ExperimentMessage::MetricLog {
                            name,
                            epoch,
                            iteration: self.iteration_count,
                            value,
                            group,
                        })
                        .unwrap();
                }
                ExperimentMessage::Log(log) => {
                    logs.push(log.clone()).unwrap();
                    self.ws_client.send(ExperimentMessage::Log(log)).unwrap();
                }
                ExperimentMessage::Error(err) => {
                    self.ws_client.send(ExperimentMessage::Error(err)).unwrap();
                }
                ExperimentMessage::Close => {
                    break;
                }
            }
        }
        self.ws_client.close().unwrap();

        WSThreadResult { logs }
    }
}

#[derive(Debug)]
pub struct ExperimentSocket {
    sender: mpsc::Sender<ExperimentMessage>,
    handle: JoinHandle<WSThreadResult>,
}

impl ExperimentSocket {
    pub fn new(ws_client: WebSocketClient, log_store: TempLogStore) -> Self {
        let (sender, receiver) = mpsc::channel();

        let thread = ExperimentWSThread::new(ws_client, receiver, log_store);
        let handle = std::thread::spawn(move || thread.run());

        Self { sender, handle }
    }

    pub fn sender(&self) -> mpsc::Sender<ExperimentMessage> {
        self.sender.clone()
    }

    pub fn close(self) -> Result<WSThreadResult, ()> {
        let _ = self.sender.send(ExperimentMessage::Close);
        self.handle.join().map_err(|_| ())
    }
}
