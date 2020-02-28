use crate::*;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::process::*;

pub struct ChildOut {
    inner: Arc<Option<Child>>,
}

pub struct ChildIn {
    inner: Arc<Option<Child>>,
}

/// splits the given child process into an owned read and a write end.
/// Returns either both ends if they are both present or the child if either is missing
pub fn split_child(child: Child) -> Result<(ChildOut, ChildIn), Child> {
    if child.stdin.is_some() && child.stdout.is_some() {
        let arc = Arc::new(Some(child));
        Ok((ChildOut { inner: arc.clone() }, ChildIn { inner: arc }))
    } else {
        Err(child)
    }
}

// pub fn unsplit_child(mut child_in: ChildIn, child_out: ChildOut) -> Child {
//     assert!(
//         std::ptr::eq(&*child_in.inner, &*child_out.inner),
//         "child values are not from same process"
//     );
//     drop(child_out);
//     Arc::get_mut(&mut child_in.inner).unwrap().take().unwrap()
// }

// the following unsafe operations are safe because the input end only accesses the read end, the write end only the write end
// and only those two can exist at the same time

unsafe fn get_pin_stdout(c: &ChildOut) -> Pin<&mut ChildStdout> {
    let ptr: *const Child = c.inner.as_ref().as_ref().unwrap();
    let child_out = (ptr as *mut Child)
        .as_mut()
        .unwrap()
        .stdout
        .as_mut()
        .unwrap();
    Pin::new_unchecked(child_out)
}

unsafe fn get_pin_stdin(c: &ChildIn) -> Pin<&mut ChildStdin> {
    let ptr: *const Child = c.inner.as_ref().as_ref().unwrap();
    let child_in = (ptr as *mut Child)
        .as_mut()
        .unwrap()
        .stdin
        .as_mut()
        .unwrap();
    Pin::new_unchecked(child_in)
}

impl tokio::io::AsyncRead for ChildOut {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        unsafe { get_pin_stdout(&self).poll_read(cx, buf) }
    }
}

impl tokio::io::AsyncWrite for ChildIn {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        unsafe { get_pin_stdin(&self).poll_write(cx, buf) }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), std::io::Error>> {
        unsafe { get_pin_stdin(&self).poll_flush(cx) }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), std::io::Error>> {
        unsafe { get_pin_stdin(&self).poll_shutdown(cx) }
    }
}
