
use crate::{error::HeatSdkError, websocket::WebSocketClient, ws_messages::WsMessage};
use std::{collections::HashMap, sync::mpsc, thread::JoinHandle};

#[derive(Debug)]
struct LogChannel {
    pub sender: mpsc::Sender<String>,
    pub receiver: mpsc::Receiver<String>,
}

impl LogChannel {
    fn new() -> LogChannel {
        let (sender, receiver) = mpsc::channel();
        LogChannel { sender, receiver }
    }

    fn send(&self, log: String) -> Result<(), HeatSdkError> {
        self.sender.send(log).map_err(|e| HeatSdkError::ClientError(e.to_string()))
    }

    fn recv(&self) -> Result<String, HeatSdkError> {
        self.receiver.recv().map_err(|e| HeatSdkError::ClientError(e.to_string()))
    }

    pub fn get_sender(&self) -> mpsc::Sender<String> {
        self.sender.clone()
    }

    pub fn split(self) -> (mpsc::Sender<String>, mpsc::Receiver<String>) {
        (self.sender, self.receiver)
    }
}

#[derive(Debug)]
struct WSHandlerResult {
    logs: Vec<String>,
}

#[derive(Debug)]
struct ExperimentWSHandler<R> {
    handler_state_sender: mpsc::Sender<Result<(), HeatSdkError>>,
    log_channel_sender: mpsc::Sender<String>,
    handle: JoinHandle<R>,
}

impl<R> ExperimentWSHandler<R> {
    pub fn get_log_sender(&self) -> mpsc::Sender<String> {
        self.log_channel_sender.clone()
    }
    
    fn join(self) -> R {
        self.handler_state_sender.send(Ok(())).unwrap();
        self.handle.join().unwrap()
    }
}

struct ExperimentCheckpointMessage {
    path: String,
    data: Vec<u8>,
}

#[derive(Debug)]
struct ExperimentCheckpointerHandler {
    handler_state_sender: mpsc::Sender<Result<(), HeatSdkError>>,
    checkpoint_channel_sender: mpsc::Sender<Vec<u8>>,
    handle: JoinHandle<()>,
}

impl ExperimentCheckpointerHandler {

    fn join(self) {
        self.handler_state_sender.send(Ok(())).unwrap();
        self.handle.join().unwrap();
    }
}

#[derive(Debug)]
pub struct Experiment {
    id: String,
    in_memory_logs: Option<Vec<String>>,
    // ws_client: WebSocketClient,
    handler: Option<ExperimentWSHandler<WSHandlerResult>>,
    // scheduler_checkpoint_handler: Option<ExperimentCheckpointerHandler>,
    // optimizer_checkpoint_handler: Option<ExperimentCheckpointerHandler>,
    // experiment_checkpoint_handler: Option<ExperimentCheckpointerHandler>,
}

impl Experiment {
    pub fn new(id: String, ws_client: WebSocketClient) -> Experiment {
        assert!(ws_client.state().is_open());

        let handler = Experiment::start(ws_client);

        Experiment {
            id,
            in_memory_logs: Some(Vec::new()),
            // ws_client: ws_client,
            handler: Some(handler),
        }
    }

    fn start(mut ws_client: WebSocketClient) -> ExperimentWSHandler<WSHandlerResult> {
        let (handler_sender, handler_receiver) = mpsc::channel::<Result<(), HeatSdkError>>();
        let (log_channel_sender, log_channel_receiver) = LogChannel::new().split();

        let handle: JoinHandle<WSHandlerResult> = std::thread::spawn(move || {
            let mut logs = Vec::new();
            loop {
                match handler_receiver.try_recv() {
                    Ok(Ok(_)) => break,
                    Ok(Err(e)) => {
                        logs.push(e.to_string());
                        ws_client.send(WsMessage::Error(e.to_string())).unwrap();
                    },
                    Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => (),
                }
                match log_channel_receiver.try_recv() {
                    Ok(log) => {
                        logs.push(log.clone());
                        ws_client.send(WsMessage::Log(log)).unwrap();
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => (),
                }
            }
            ws_client.close().unwrap();

            WSHandlerResult { logs: logs }
        });

        ExperimentWSHandler {
            handler_state_sender: handler_sender,
            log_channel_sender: log_channel_sender,
            handle,
        }
    }

    pub fn add_log(&mut self, log: String) -> Result<(), HeatSdkError> {
        // self.in_memory_logs.push(log.clone());
        // self.ws_client.send(WsMessage::Log(log))?;

        Ok(())
    }

    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn try_logs(&self) -> Result<Vec<String>, HeatSdkError> {
        if let Some(logs) = &self.in_memory_logs {
            Ok(logs.clone())
        } else {
            Err(HeatSdkError::ClientError("Logs not yet available".to_string()))
        }
    }

    pub fn get_log_sender(&self) -> Result<mpsc::Sender<String>, HeatSdkError> {
        if let Some(handler) = &self.handler {
            Ok(handler.get_log_sender())
        } else {
            Err(HeatSdkError::ClientError("Experiment handling thread not started".to_string()))
        }
    }

    pub fn stop(&mut self) {
        // self.ws_client.close().unwrap();
        
        if let Some(handler) = self.handler.take() {
            let result = handler.join();
            self.in_memory_logs.replace(result.logs);
        }
    }
    

}

impl Drop for Experiment {
    fn drop(&mut self) {
        // self.ws_client.close().unwrap();
        self.stop();
    }
}
