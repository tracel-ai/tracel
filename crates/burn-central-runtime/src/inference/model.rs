use std::any::Any;
use std::fmt::{Debug, Display, Formatter};
use std::thread::JoinHandle;

pub struct ModelHost<M> {
    accessor: ModelAccessor<M>,
    abort_tx: crossbeam::channel::Sender<()>,
    join_handle: Option<JoinHandle<M>>,
}

type BoxAny = Box<dyn Any + Send>;

enum Msg<M> {
    Call {
        f: Box<dyn FnOnce(&mut M) + Send>,
        done: crossbeam::channel::Sender<()>,
    },
    CallRet {
        f: Box<dyn FnOnce(&mut M) -> BoxAny + Send>,
        ret: crossbeam::channel::Sender<BoxAny>,
    },
}

impl<M: 'static + Send> ModelHost<M> {
    pub fn spawn(model: M) -> Self {
        let (abort_tx, abort_rx) = crossbeam::channel::unbounded::<()>();
        let (tx, rx) = crossbeam::channel::unbounded::<Msg<M>>();
        let join_handle = std::thread::spawn(move || {
            let mut m = model;
            loop {
                crossbeam::channel::select! {
                    recv(rx) -> msg => {
                        match msg {
                            Ok(Msg::Call { f, done }) => {
                                f(&mut m);
                                let _ = done.send(());
                            }
                            Ok(Msg::CallRet { f, ret }) => {
                                let r = f(&mut m);
                                let _ = ret.send(r);
                            }
                            Err(_) => break,
                        }
                    }
                    recv(abort_rx) -> _ => {
                        break;
                    }
                }
            }
            m
        });
        Self {
            accessor: ModelAccessor { tx },
            abort_tx,
            join_handle: Some(join_handle),
        }
    }

    pub fn accessor(&self) -> ModelAccessor<M> {
        self.accessor.clone()
    }

    pub fn into_model(mut self) -> M {
        let _ = self.abort_tx.send(());

        self.join_handle
            .take()
            .expect("Should have join handle")
            .join()
            .expect("Thread should not panic")
    }
}

impl<M> std::ops::Deref for ModelHost<M> {
    type Target = ModelAccessor<M>;

    fn deref(&self) -> &Self::Target {
        &self.accessor
    }
}

impl<M> Drop for ModelHost<M> {
    fn drop(&mut self) {
        let _ = self.abort_tx.send(());
        let _ = self.join_handle.take().unwrap().join();
    }
}

pub struct ModelAccessor<M> {
    tx: crossbeam::channel::Sender<Msg<M>>,
}

impl<M: Debug> Debug for ModelAccessor<M> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let debug_str = self.with(|m| format!("{m:?}"));
        write!(f, "{debug_str}")
    }
}

impl<M: Display> Display for ModelAccessor<M> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let display_str = self.with(|m| format!("{m}"));
        write!(f, "{display_str}")
    }
}

impl<M> Clone for ModelAccessor<M> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<M> ModelAccessor<M> {
    pub fn with<R: Send + 'static>(&self, f: impl FnOnce(&mut M) -> R + Send + 'static) -> R {
        let (ret_tx, ret_rx) = crossbeam::channel::bounded(1);
        let _ = self.tx.send(Msg::CallRet {
            f: Box::new(move |m| Box::new(f(m)) as BoxAny),
            ret: ret_tx,
        });
        let r = ret_rx.recv().unwrap();
        *r.downcast::<R>().unwrap()
    }

    pub fn fire(&self, f: impl FnOnce(&mut M) + Send + 'static) {
        let (done_tx, done_rx) = crossbeam::channel::bounded(1);
        let _ = self.tx.send(Msg::Call {
            f: Box::new(f),
            done: done_tx,
        });
        let _ = done_rx.recv();
    }
}
