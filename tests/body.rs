use http_fancy::body::{Collect, CollectError};
use core::future::Future;
use core::pin::Pin;
use core::task;

mod waker {
    use core::{task, mem};

    static VTABLE: task::RawWakerVTable = task::RawWakerVTable::new(should_not_clone, action, action, noop);

    #[cold]
    #[inline(never)]
    fn should_not_clone(_: *const()) -> task::RawWaker {
        panic!("Impossible Waker Clone");
    }

    fn noop(_: *const()) {
    }

    unsafe fn action(callback: *const ()) {
        let func: fn() = mem::transmute(callback);
        func()
    }

    pub fn create(data: fn()) -> task::Waker {
        unsafe {
            task::Waker::from_raw(task::RawWaker::new(data as *const (), &VTABLE))
        }
    }
}

fn should_not_call_waker() {
    panic!("should not call waker");
}

#[track_caller]
fn call_future_once<T: Future + Unpin>(mut fut: T) -> T::Output {
    let location = core::panic::Location::caller();

    let waker = waker::create(should_not_call_waker);
    let mut ctx = task::Context::from_waker(&waker);
    match Future::poll(Pin::new(&mut fut), &mut ctx) {
        task::Poll::Ready(result) => result,
        task::Poll::Pending => panic!("unexpected pending from {location}")
    }
}

#[test]
fn should_collect_small_body() {
    let body = "12".to_owned();
    let result = Collect::<2, _, _>::new(body, Vec::new());
    match call_future_once(result) {
        Ok(data) => assert_eq!(data, b"12"),
        Err(error) => panic!("Unexpected error: {error}"),
    }
}

#[test]
fn should_overflow_on_limit() {
    let body = "12".to_owned();
    let result = Collect::<1, _, _>::new(body, Vec::new());
    match call_future_once(result) {
        Err(CollectError::Overflow) => (),
        Err(error) => panic!("Unexpected error: {error}"),
        Ok(data) => panic!("Unexpected result: {:?}", data),
    }
}

#[cfg(feature = "compress")]
#[test]
fn should_decompress_zstd() {
    let body: http_fancy::body::Body = zstd::bulk::compress(b"123456789", 9).expect("To encode").into();

    let result = Collect::<100, _, _>::new(body, http_fancy::body::DecompressCollector::new());
    match call_future_once(result) {
        Ok(data) => assert_eq!(data, b"123456789"),
        Err(error) => panic!("Unexpected error: {error}"),
    }
}
