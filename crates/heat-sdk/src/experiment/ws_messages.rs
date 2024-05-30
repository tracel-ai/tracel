use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Split {
    Train,
    Val,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum WsMessage {
    MetricLog {
        name: String,
        epoch: usize,
        value: f64,
        split: Split,
    },
    Log(String),
    Error(String),
    Close,
}

impl<S: Into<String> + Clone> From<S> for WsMessage {
    fn from(msg: S) -> Self {
        let deser_msg: Result<WsMessage, _> = serde_json::from_str(&msg.clone().into());
        match deser_msg {
            Ok(msg) => msg,
            Err(_) => WsMessage::Error(format!("Invalid message: {}", msg.into())),
        }
    }
}

impl ToString for WsMessage {
    fn to_string(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}
