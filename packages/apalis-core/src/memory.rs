use crate::{
    backend::Backend,
    mq::MessageQueue,
    poller::Poller,
    poller::{controller::Controller, stream::BackendStream},
    request::{Request, RequestStream},
    worker::{self, Worker},
};
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    Stream, StreamExt,
};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tower::layer::util::Identity;

#[derive(Debug)]
/// An example of the basics of a backend
pub struct MemoryStorage<T> {
    /// Required for [Poller] to control polling.
    controller: Controller,
    /// This would be the backend you are targeting, eg a connection poll
    inner: MemoryWrapper<T>,
}
impl<T> MemoryStorage<T> {
    /// Create a new in-memory storage
    pub fn new() -> Self {
        Self {
            controller: Controller::new(),
            inner: MemoryWrapper::new(),
        }
    }
}

impl<T> Default for MemoryStorage<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for MemoryStorage<T> {
    fn clone(&self) -> Self {
        Self {
            controller: self.controller.clone(),
            inner: self.inner.clone(),
        }
    }
}

/// In-memory queue that implements [Stream]
#[derive(Debug)]
pub struct MemoryWrapper<T> {
    sender: Sender<Request<T, ()>>,
    receiver: Arc<futures::lock::Mutex<Receiver<Request<T, ()>>>>,
}

impl<T> Clone for MemoryWrapper<T> {
    fn clone(&self) -> Self {
        Self {
            receiver: self.receiver.clone(),
            sender: self.sender.clone(),
        }
    }
}

impl<T> MemoryWrapper<T> {
    /// Build a new basic queue channel
    pub fn new() -> Self {
        let (sender, receiver) = channel(100);

        Self {
            sender,
            receiver: Arc::new(futures::lock::Mutex::new(receiver)),
        }
    }
}

impl<T> Default for MemoryWrapper<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Stream for MemoryWrapper<T> {
    type Item = Request<T, ()>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(mut receiver) = self.receiver.try_lock() {
            receiver.poll_next_unpin(cx)
        } else {
            Poll::Pending
        }
    }
}

// MemoryStorage as a Backend
impl<T: Send + 'static + Sync, Res> Backend<Request<T, ()>, Res> for MemoryStorage<T> {
    type Stream = BackendStream<RequestStream<Request<T, ()>>>;

    type Layer = Identity;

    fn poll<Svc>(self, _worker: &Worker<worker::Context>) -> Poller<Self::Stream> {
        let stream = self.inner.map(|r| Ok(Some(r))).boxed();
        Poller {
            stream: BackendStream::new(stream, self.controller),
            heartbeat: Box::pin(futures::future::pending()),
            layer: Identity::new(),
            _priv: (),
        }
    }
}

impl<Message: Send + 'static + Sync> MessageQueue<Message> for MemoryStorage<Message> {
    type Error = ();
    async fn enqueue(&mut self, message: Message) -> Result<(), Self::Error> {
        self.inner
            .sender
            .try_send(Request::new(message))
            .map_err(|_| ())?;
        Ok(())
    }

    async fn dequeue(&mut self) -> Result<Option<Message>, ()> {
        Ok(self
            .inner
            .receiver
            .lock()
            .await
            .next()
            .await
            .map(|r| r.args))
    }

    async fn size(&mut self) -> Result<usize, ()> {
        Ok(self.inner.receiver.lock().await.size_hint().0)
    }
}
