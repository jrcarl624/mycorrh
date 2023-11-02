pub mod event;
pub mod message;
pub mod state;


// Soon...
// macro_rules! event_loop {
//     () => {};
// }


#[cfg(test)]
mod test {
    use std::fmt::{Display, Formatter};
    use std::sync::Arc;
    use tokio::sync::{mpsc, RwLock};
    use tokio::task::JoinHandle;

    use super::*;

    // use state::State;
    use event::EventLoop;
    use crate::message::{MessageEmitter, MessageHandle};

    #[derive(Debug)]
    enum TestMessage {
        Increment,
        Decrement,
        GetCurrent,
        GetError,
    }

    #[derive(Debug)]
    enum TestResponse {
        Current(usize),
    }

    #[derive(Debug)]
    struct TestClose;

    impl Display for TestClose {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(f, "TestClose")
        }
    }


    #[derive(Debug, PartialEq)] // PartialEq is required for the test, not a normal requirement
    enum TestError {
        TestError,
        FailedToIncrement,
        FailedToDecrement,
        FailedToGetCurrent,
        AlreadyClosed,
    }

    impl Display for TestError {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match self {
                TestError::TestError =>
                    write!(f, "Error used to test errors"),

                TestError::FailedToIncrement =>
                    write!(f, "Failed to increment number"),

                TestError::FailedToDecrement =>

                    write!(f, "Failed to decrement number"),

                TestError::FailedToGetCurrent =>
                    write!(f, "Failed to get current number"),

                TestError::AlreadyClosed =>
                    write!(f, "Already closed"),
            }
        }
    }

    impl std::error::Error for TestError {}

    type TestMessageType = message::MessageType<TestMessage, TestResponse, TestClose, TestError>;

    #[derive(Clone)]
    struct TestEventLoop {
        //   state: Arc<TestState>,
        event_handle: Arc<RwLock<JoinHandle<Result<TestClose, TestError>>>>,
        message_handle: MessageHandle<TestMessage, TestResponse, TestClose, TestError>,
    }

    impl TestEventLoop {
        async fn new() -> Self {
            let (message_tx, mut message_rx) = mpsc::channel::<TestMessageType>(256);

            // let thread_state = Arc::new(TestState {});
            // let state = Arc::clone(&thread_state);


            let event_loop = tokio::spawn(async move {
                //let state = thread_state;

                let mut message_rx = message_rx;
                //let mut reference_count: usize = 0;

                let mut num = 0;

                loop {
                    tokio::select! {

                            m = message_rx.recv() => {
                                if let Some(m) = m {
                                    match m {
                                        TestMessageType::Event(mut message) => {
                                            match message.message() {
                                                    // Notifications
                                                    TestMessage::Decrement => {
                                                        num-= 1
                                                    }
                                                    TestMessage::Increment => {
                                                        num+= 1
                                                    }
                                                    // Messages
                                                    TestMessage::GetCurrent=> {
                                                        match message.callback_ok(TestResponse::Current(num)) {
                                                            Ok(_) => {}
                                                            Err(_) => {
                                                                #[cfg(debug_assertions)]
                                                                dbg!("Failed to send TestResponse::Current");
                                                            }
                                                }
                                                    }
                                                    TestMessage::GetError => {
                                                      match   message.callback_err(TestError::TestError) {
                                                            Ok(_) => {}
                                                            Err(_) => {
                                                                #[cfg(debug_assertions)]
                                                                dbg!("Failed to send TestError::TestError");
                                                            }
                                                }
                                                    }

                                                }
                                        },
                                        TestMessageType::Close(reason) => {
                                            #[cfg(debug_assertions)]
                                            dbg!("Closing TestEventLoop: {}", reason.to_string());
                                            break Ok(reason)
                                        }
                                        _ => todo!()


                                    }

                                }
                            }
                        }
                }
            });
            Self {
                //state,
                event_handle: Arc::new(RwLock::new(event_loop)),
                message_handle: MessageHandle::new(message_tx),
            }
        }

        pub async fn increment(&self) -> Result<(), TestError> {
            match self.message_handle.send_notification(TestMessage::Increment, "increment").await {
                Ok(_) => Ok(()),
                Err(_e) => Err(TestError::FailedToIncrement)
            }
        }

        pub async fn decrement(&self) -> Result<(), TestError> {
            match self.message_handle.send_notification(TestMessage::Decrement, "decrement").await {
                Ok(_) => Ok(()),
                Err(_e) => Err(TestError::FailedToDecrement)
            }
        }

        pub async fn current(&self) -> Result<usize, TestError> {
            // if we know the response here could we convert on the fly?
            match self.message_handle.send_message(TestMessage::GetCurrent, "current").await {
                Ok(res) => {
                    match res {
                        TestResponse::Current(i) => Ok(i),
                        //_ => unreachable!()
                    }
                }
                Err(_e) => Err(TestError::FailedToGetCurrent)
            }
        }
        // TODO: Make a wrapper proc macro for this, should

        pub async fn error_test(&self) -> Result<(), TestError> {
            match self.message_handle.send_message(TestMessage::GetError, "error_test").await {
                Ok(_) => unreachable!(),
                Err(_e) => Err(TestError::TestError)
            }
        }
    }

    // #[derive(Debug)]
    // struct TestState {
    // }
    // impl State for TestEventLoop {
    //     type State = TestState;
    //     fn state(&self) -> &Arc<Self::State> {
    //         &self.state
    //     }
    // }

    impl MessageEmitter for TestEventLoop {
        type Message = TestMessage;
        type Response = TestResponse;

        fn message_handle(&self) -> &MessageHandle<Self::Message, TestResponse, TestClose, TestError> {
            &self.message_handle
        }
    }

    #[async_trait::async_trait]
    impl EventLoop for TestEventLoop {
        type Close = TestClose;
        type Err = TestError;

        fn event_loop_join_handle(&self) -> &Arc<RwLock<JoinHandle<Result<Self::Close, Self::Err>>>> {
            &self.event_handle
        }

        fn is_closed(&self) -> bool {
            self.message_handle.is_closed()
        }

        async fn close(&self, reason: Self::Close) -> Result<(), Self::Err> {
            self.message_handle.close(reason).await.map_err(|_| TestError::AlreadyClosed)
        }
    }


    /// Returns how many successful increments were made
    fn spawn_test_task(test_ref: TestEventLoop, inc: u32) -> JoinHandle<Result<u32, TestError>>
    {
        tokio::task::spawn(async move {
            let test_event_loop_ref = test_ref;

            let mut incs = 0;

            for _ in 0..inc {
                match test_event_loop_ref.increment().await {
                    Ok(_) => incs += 1,
                    Err(e) => {
                        println!("Failed to increment: {:?}", e);
                    }
                }
            }

            Ok(incs)
        })
    }

    #[tokio::test]
    async fn test_event_system() {
        // As the constructor returns a reference to the event loop, it is not directly the event loop
        let main_task = TestEventLoop::new().await;

        let task_1 = spawn_test_task(main_task.clone(), 1000);

        let task_2 = spawn_test_task(main_task.clone(), 1000);


        let (res_1, res_2) = tokio::join!(task_1,task_2);

        print!("Task 1: {:?}, Task 2: {:?}", res_1, res_2);

        let current = main_task.current().await.unwrap();


        assert_eq!(current, 2000);
    }

    #[tokio::test]
    async fn test_send_error() {
        // As the constructor returns a reference to the event loop, it is not directly the event loop
        let main_task = TestEventLoop::new().await;


        let task_1 = spawn_test_task(main_task.clone(), 1000);


        let (res_1) = tokio::join!(task_1);


        let current = main_task.current().await.unwrap();


        assert_eq!(current, 2000);
    }
//  test the error system

    #[tokio::test]
    async fn test_error_system() {
        let test = TestEventLoop::new().await;

        let err = test.error_test().await;

        assert_eq!(err, Err(TestError::TestError));
    }

    #[tokio::test]
    async fn test_close() {
        let test = TestEventLoop::new().await;

        let task_1 = test.clone();


        assert_eq!(test.close(TestClose).await, Ok(()));

        assert_eq!(task_1.close(TestClose).await, Err(TestError::AlreadyClosed));
    }
}
