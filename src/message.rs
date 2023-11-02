use std::sync::atomic::{ AtomicU16, Ordering};
use tokio::sync::{
    mpsc::{Sender, UnboundedReceiver, UnboundedSender},
    Mutex,
};

use crate::event::EventLoop;


// Make sync and also async


// TODO: We should store how many refs are held to the event loop, the id of the ref should be stored in the ref itself and contact state when it opens and close
// have it auto close when there are no references to the event loop as an option
// as it opens with one reference, it should close with 0 references instead of having to call close on the children
// there would be a small overhead checking if there is no refs, but when a child ref closes it can send a message stating such
// and the event loop can check if there are no refs and close itself when a ref closes instead of having to call close on the children to close the loop, we can have a system where a loop closes safely when there are no refs to it
// when the ref is dropped it should call that it is closing so there is no need for user intervention when it is mistakenly dropped
// this will have to be looked into further

// note for future, we have to create the count in the event loop constructor, and pass it in here where the loop can see the count,we  also have to figure out the best
// way to close the inner event loop from the outside loops if needed, but safely so there needs to be a function that allows for safe shutdown and of course hard shutdown, maybe an interface...


/// The MessageEmitter trait defines the methods required for an event emitter that sends messages.
/// Implementers of this trait should be able to send a message and handle the response.
/// It contains associated types for Message, Response, Metadata, ensuring that these types are
/// Send, Sync, and have proper formatting.
#[async_trait::async_trait]
pub trait MessageEmitter: EventLoop
    where
        Self: Send + Sync + Clone,
{
    type Message: Send + Sync + std::fmt::Debug;
    type Response: Send + Sync + std::fmt::Debug;

    fn message_handle(
        &self,
    ) -> &MessageHandle<Self::Message, Self::Response, Self::Close, Self::Err>;
}

/// The MessageHandle struct provides a convenient way to manage the message transmission and reception process.
/// It handles message correlation and provides methods for sending and receiving messages.
#[derive(Debug)]
pub struct MessageHandle<Message, Response, Close, Err>
    where
        Message: Send + Sync + std::fmt::Debug,
        Response: Send + Sync + std::fmt::Debug,
        Close: Send + Sync + std::fmt::Debug + std::fmt::Display,
        Err: Send + Sync + std::error::Error,
{
    correlation_id: AtomicU16,
    message_tx: Sender<MessageType<Message, Response, Close, Err>>,
    handle_tx: UnboundedSender<MessageResult<Response, Err>>,
    handle_rx: Mutex<UnboundedReceiver<MessageResult<Response, Err>>>,
}

// Is used to manage message transmission and receive results.
impl<Message, Response, Close, Err> MessageHandle<Message, Response, Close, Err>
    where
        Message: Send + Sync + std::fmt::Debug,
        Response: Send + Sync + std::fmt::Debug,
        Close: Send + Sync + std::fmt::Debug + std::fmt::Display,
        Err: Send + Sync + std::error::Error,
{
    /// This is used when a clone occurs
    pub fn new(mut message_tx: Sender<MessageType<Message, Response, Close, Err>>) -> Self {
        let (handle_tx, rx) = tokio::sync::mpsc::unbounded_channel();


        Self {
            message_tx,
            correlation_id: AtomicU16::new(0),
            handle_tx,
            handle_rx: Mutex::new(rx),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.message_tx.is_closed()
    }

    pub async fn close(&self, reason: Close) -> Result<(), MessageError<Close, Err>> {
        let close = self.message_tx.send(MessageType::Close(reason)).await.map_err(|e| match e.0 {
            MessageType::Close(reason) => MessageError::SendError(reason),
            _ => unreachable!(),
        });

        while !self.is_closed() {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        close
    }

    pub(self) fn next_correlation_id(&self) -> u16 {
        self.correlation_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Sends a message and waits for a response. This is slightly more expensive than send_notification.
    pub async fn send_message<'a>(
        &'a self,
        message: Message,
        origin_id: &'static str,
    ) -> Result<Response, MessageError<MessageEvent<Message, Response, Err>, Err>> {
        let correlation_id = self.next_correlation_id();

        // create the message event
        let msg =
            MessageEvent::new_message(origin_id, correlation_id, message, self.handle_tx.clone());

        // send the message to the message channel
        self.message_tx
            .send(MessageType::Event(msg))
            .await
            .map_err(|e| match e.0 {
                MessageType::Event(msg) => {
                    if self.is_closed() {
                        MessageError::MessageChannelClosed(msg)
                    } else {
                        MessageError::SendError(msg)
                    }

                    // MessageError::SendError(msg)
                }
                _ => unreachable!(),
            })?;

        // Acquire the lock outside of the loop, as it sends the messages that do not match into the queue until it finds the correct one.
        // If the lock is acquired inside the loop, it might deadlock if other functions are called that require the lock.
        let mut rx = self.handle_rx.lock().await;

        loop {
            match rx.recv().await {
                Some(result) => {
                    if result.check_correlation(correlation_id) {
                        drop(rx);
                        return result.result().map_err(MessageError::MessageError);
                    } else {
                        let id_pair = result.id_pair();
                        self.handle_tx.send(result).map_err(|_| {
                            MessageError::MisdirectedResultSendError {
                                origin: (origin_id, correlation_id),
                                result: id_pair,
                            }
                        })?;
                    }
                }
                None => {
                    drop(rx);
                    return Err(MessageError::RecvError(origin_id, correlation_id));
                }
            }
        }
    }

    /// Sends a message without waiting for a response
    pub async fn send_notification<'a>(
        &'a self,
        message: Message,
        origin_id: &'static str,
    ) -> Result<(), MessageError<MessageEvent<Message, Response, Err>, Err>> {
        let correlation_id = self.next_correlation_id();

        // create the message event
        let msg = MessageEvent::new_notification(origin_id, correlation_id, message);

        // send the message to the message channel
        self.message_tx
            .send(MessageType::Event(msg))
            .await
            .map_err(|e| match e.0 {
                MessageType::Event(msg) => MessageError::SendError(msg),
                _ => unreachable!(),
            })?;

        Ok(())
    }

    // pub async fn close_handle(self) {
    //     match self.message_tx.send(MessageType::Unsubscribe).await {
    //         _ => {}
    //     }
    // }
}


impl<T, R, C, E> Clone for MessageHandle<T, R, C, E>
    where
        T: Send + Sync + std::fmt::Debug,
        R: Send + Sync + std::fmt::Debug,
        C: Send + Sync + std::fmt::Debug + std::fmt::Display,
        E: Send + Sync + std::error::Error,
{
    fn clone(&self) -> Self {
        MessageHandle::new(self.message_tx.clone())
    }
}


/// MessageType is an enum representing different types of messages that can be sent
/// and received within the message handling system. It can contain a message event,
/// a message result, or a notification that a listener has closed.
pub enum MessageType<T, R, C, E>
    where
        T: Send + Sync + std::fmt::Debug,
        R: Send + Sync + std::fmt::Debug,
        C: Send + Sync + std::fmt::Debug + std::fmt::Display,
        E: Send + Sync + std::error::Error,
{
    /// The MessageEvent is sent from the message handle to the event loop
    Event(MessageEvent<T, R, E>),
    /// The MessageResult is sent from the event loop to the message handle
    EventResponse(MessageResult<R, E>),
    /// Closes The event loop
    Close(C),
}

/// MessageError is an enum representing various error cases that can occur within
/// the message handling system. It can represent errors during sending, receiving,
/// handling misdirected returns, or processing messages.
#[derive(Debug)]
pub enum MessageError<Message, Err>
    where
        Message: Send + Sync + std::fmt::Debug,
        Err: Send + Sync + std::error::Error,
{
    MessageChannelClosed(Message),
    MessageError(Err),
    MisdirectedResultSendError {
        origin: (&'static str, u16),
        result: (&'static str, u16),
    },
    RecvError(&'static str, u16),
    ResultNotExpected(&'static str),
    SendError(Message),
}

impl<T, E> std::error::Error for MessageError<T, E>
    where
        T: Send + Sync + std::fmt::Debug,
        E: Send + Sync + std::error::Error,
{}

impl<T, E> std::fmt::Display for MessageError<T, E>
    where
        T: Send + Sync + std::fmt::Debug,
        E: Send + Sync + std::error::Error,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageError::SendError(t) => {
                write!(f, "SendError: {t:?}")
            }
            MessageError::MessageChannelClosed(m) => {
                write!(f, "ChannelClosed with message: {m:?}")
            }

            MessageError::MisdirectedResultSendError {
                origin: (origin_id, origin_correlation_id),
                result: (result_id, result_correlation_id),
            } => {
                write!(
                    f,
                    "MisdirectedResultSendError: origin_id: {}, origin_correlation_id: {}, return_id: {}, return_correlation_id: {}",
                    origin_id, origin_correlation_id, result_id, result_correlation_id
                )
            }

            MessageError::RecvError(o_id, c_id) => {
                write!(f, "RecvError: origin_id {}, correlation_id {}", o_id, c_id)
            }

            MessageError::MessageError(e) => {
                write!(f, "MessageErr: {}", e)
            }

            MessageError::ResultNotExpected(origin_id) => {
                write!(f, "ResultNotExpected: origin_id {}", origin_id)
            }
        }
    }
}

#[derive(Debug)]
pub struct MessageEvent<T, R, E>
    where
        T: Send + Sync + std::fmt::Debug,
        R: Send + Sync + std::fmt::Debug,
        E: Send + Sync + std::error::Error,
{
    origin: &'static str,
    pub message: T,
    correlation_id: u16,
    callback: Option<UnboundedSender<MessageResult<R, E>>>,
}

impl<T, R, E> MessageEvent<T, R, E>
    where
        T: Send + Sync + std::fmt::Debug,
        R: Send + Sync + std::fmt::Debug,
        E: Send + Sync + std::error::Error,
{
    pub fn new_message(
        origin: &'static str,
        correlation_id: u16,
        message: T,
        callback: UnboundedSender<MessageResult<R, E>>,
    ) -> Self {
        Self {
            origin,
            message,
            callback: Some(callback),
            correlation_id,
        }
    }

    pub fn new_notification(id: &'static str, correlation_id: u16, message: T) -> Self {
        Self {
            origin: id,
            message,
            callback: None,
            correlation_id,
        }
    }

    pub fn message(&mut self) -> &mut T {
        &mut self.message
    }

    pub fn into_message(self) -> T {
        self.message
    }

    pub fn correlation_id(&self) -> &u16 {
        &self.correlation_id
    }

    pub fn origin(&self) -> &'static str {
        &self.origin
    }

    pub fn callback_ok(self, data: R) -> Result<(), MessageError<MessageResult<R, E>, E>> {
        self.callback
            .unwrap()
            .send(MessageResult::new_ok(
                data,
                self.origin,
                self.correlation_id,
            ))
            .map_err(|e| MessageError::SendError(e.0))
    }

    pub fn try_callback_ok(self, data: R) -> Result<(), MessageError<MessageResult<R, E>, E>> {
        match &self.callback {
            Some(cb) => cb
                .send(MessageResult::new_ok(
                    data,
                    self.origin,
                    self.correlation_id,
                ))
                .map_err(|e| MessageError::SendError(e.0)),
            None => Err(MessageError::ResultNotExpected(self.origin)),
        }
    }

    pub fn callback_err(self, err: E) -> Result<(), MessageError<MessageResult<R, E>, E>> {
        self.callback
            .unwrap()
            .send(MessageResult::new_err(
                err,
                self.origin,
                self.correlation_id,
            ))
            .map_err(|e| MessageError::SendError(e.0))
    }

    pub fn try_callback_err(self, data: E) -> Result<(), MessageError<MessageResult<R, E>, E>> {
        match &self.callback {
            Some(cb) => cb
                .send(MessageResult::new_err(
                    data,
                    self.origin,
                    self.correlation_id,
                ))
                .map_err(|e| MessageError::SendError(e.0)),
            None => Err(MessageError::ResultNotExpected(self.origin)),
        }
    }
}

/// MessageResult is used to represent the result of processing a MessageEvent. It
/// contains the correlation ID, the result (either a successful value or an error), and
/// the identifier of the message origin.
#[derive(Debug)]
pub struct MessageResult<R, E>(&'static str, u16, Result<R, E>)
    where
        R: Send + Sync + std::fmt::Debug,
        E: Send + Sync + std::error::Error;

impl<R, E> MessageResult<R, E>
    where
        R: Send + Sync + std::fmt::Debug,
        E: Send + Sync + std::error::Error,
{
    pub fn new_err(err: E, origin: &'static str, correlation_id: u16) -> Self {
        Self(origin, correlation_id, Err(err))
    }

    pub fn new_ok(data: R, id: &'static str, correlation_id: u16) -> Self {
        Self(id, correlation_id, Ok(data))
    }

    pub fn result(self) -> Result<R, E> {
        self.2
    }

    pub fn origin(&self) -> &'static str {
        self.0
    }

    pub fn check_correlation(&self, id: u16) -> bool {
        self.1 == id
    }

    pub fn correlation_id(&self) -> u16 {
        self.1
    }

    pub fn id_pair(&self) -> (&'static str, u16) {
        (self.0, self.1)
    }
}
