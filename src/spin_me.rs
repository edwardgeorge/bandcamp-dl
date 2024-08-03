use std::{borrow::Cow, future::Future, pin::Pin, task::Poll, time::Duration};

use indicatif::{MultiProgress, ProgressBar};

pub struct SpinMe<A> {
    future: Option<Pin<Box<A>>>,
    spinner: ProgressBar,
}

impl<A, B> From<B> for SpinMe<A>
where
    B: Into<Cow<'static, str>>,
{
    fn from(value: B) -> Self {
        Self::new(value, None)
    }
}

impl<A> SpinMe<A> {
    pub fn new<S>(msg: S, multi: Option<&mut MultiProgress>) -> Self
    where
        S: Into<Cow<'static, str>>,
    {
        let mut spinner = ProgressBar::new_spinner()
            .with_style(crate::spin_style())
            .with_message(msg);
        if let Some(m) = multi {
            spinner = m.add(spinner);
        }
        spinner.enable_steady_tick(Duration::from_millis(100));
        SpinMe {
            future: None,
            spinner,
        }
    }
    pub fn in_multi(mut self, multi: &mut MultiProgress) -> Self {
        self.spinner = multi.add(self.spinner);
        self
    }
    pub fn from_str(msg: &str) -> Self {
        Self::new(msg.to_string(), None)
    }
    pub fn with_task(mut self, fut: A) -> Self {
        self.future = Some(Box::pin(fut));
        self
    }
}
impl<A: Future> SpinMe<A> {
    pub async fn with_result<F>(self, f: F) -> A::Output
    where
        F: FnOnce(&A::Output, ProgressBar),
    {
        let r = self.future.expect("").await;
        self.spinner.finish();
        f(&r, self.spinner);
        r
    }
    pub async fn try_join<B, C>(self) -> A::Output
    where
        A: Future<Output = Result<B, C>>,
    {
        self.try_with_result(|_, _| ()).await
    }
    pub async fn join(self) -> A::Output {
        self.with_result(|_, _| ()).await
    }
    pub async fn try_with_result<F, B, C>(self, f: F) -> A::Output
    where
        A: Future<Output = Result<B, C>>,
        F: FnOnce(&B, ProgressBar),
    {
        self.with_result(|r, s| match &r {
            Ok(v) => f(v, s),
            Err(_) => {
                // s.abandon();
            }
        })
        .await
    }
}

impl<A> Future for SpinMe<A>
where
    A: Future,
{
    type Output = A::Output;
    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().future.as_mut() {
            Some(f) => match f.as_mut().poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(res) => Poll::Ready(res),
            },
            None => panic!("Awaiting a SpinMe with no future"),
        }
    }
}
