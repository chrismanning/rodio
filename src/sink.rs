use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::stream::{OutputStreamHandle, PlayError};
use crate::{queue, source::Done, Sample, Source};

pub trait SinkAppender {
    /// Appends a sound to the queue of sounds to play.
    fn append<Src>(&self, source: Src)
    where
        Src: Source + Send + 'static,
        Src::Item: Sample,
        Src::Item: Send;
}

pub trait SinkControl {
    /// Gets the volume of the sound.
    ///
    /// The value `1.0` is the "normal" volume (unfiltered input). Any value other than 1.0 will
    /// multiply each sample by this value.
    fn volume(&self) -> f32;
    /// Changes the volume of the sound.
    ///
    /// The value `1.0` is the "normal" volume (unfiltered input). Any value other than `1.0` will
    /// multiply each sample by this value.
    fn set_volume(&self, value: f32);
    /// Resumes playback of a paused sink.
    ///
    /// No effect if not paused.
    fn play(&self);
    /// Pauses playback of this sink.
    ///
    /// No effect if already paused.
    ///
    /// A paused sink can be resumed with `play()`.
    fn pause(&self);
    /// Gets if a sink is paused
    ///
    /// Sinks can be paused and resumed using `pause()` and `play()`. This returns `true` if the
    /// sink is paused.
    fn is_paused(&self) -> bool;
    /// Stops the sink by emptying the queue.
    fn stop(&mut self);
    /// Destroys the sink without stopping the sounds that are still playing.
    fn detach(self);
    /// Sleeps the current thread until the sound ends.
    fn sleep_until_end(&self);
    /// Returns true if this sink has no more sounds to play.
    fn empty(&self) -> bool;
    /// Returns the number of sounds currently in the queue.
    fn len(&self) -> usize;
}

/// Handle to an device that outputs sounds.
///
/// Dropping the `Sink` stops all sounds. You can use `detach` if you want the sounds to continue
/// playing.
pub struct Sink<S: Sample + Send + 'static> {
    queue_tx: Arc<queue::SourcesQueueInput<S>>,
    sleep_until_end: Mutex<Option<Receiver<()>>>,

    controls: Arc<Controls>,
    sound_count: Arc<AtomicUsize>,

    detached: bool,
}

struct Controls {
    pause: AtomicBool,
    volume: Mutex<f32>,
    stopped: AtomicBool,
}

impl Sink<f32> {
    /// Builds a new `Sink`, beginning playback on a stream.
    #[inline]
    pub fn try_new(stream: &OutputStreamHandle) -> Result<Sink<f32>, PlayError> {
        let (sink, queue_rx) = Sink::new_idle();
        stream.play_raw(queue_rx)?;
        Ok(sink)
    }
}

impl<S: Sample + Send + 'static> Sink<S> {
    /// Builds a new `Sink`.
    #[inline]
    pub fn new_idle() -> (Sink<S>, queue::SourcesQueueOutput<S>) {
        let controls = Arc::new(Controls {
            pause: AtomicBool::new(false),
            volume: Mutex::new(1.0),
            stopped: AtomicBool::new(false),
        });
        Self::with_controls(controls)
    }

    #[inline]
    fn with_controls(controls: Arc<Controls>) -> (Sink<S>, queue::SourcesQueueOutput<S>) {
        let (queue_tx, queue_rx) = queue::queue(true);

        let sink = Sink {
            controls,
            queue_tx,
            sleep_until_end: Mutex::new(None),
            sound_count: Arc::new(AtomicUsize::new(0)),
            detached: false,
        };
        (sink, queue_rx)
    }
}

impl<S: Sample + Send + 'static> SinkAppender for Sink<S> {
    #[inline]
    fn append<Src>(&self, source: Src)
    where
        Src: Source + Send + 'static,
        Src::Item: Sample,
        Src::Item: Send,
    {
        let controls = self.controls.clone();

        let source = source
            .pausable(false)
            .amplify(1.0)
            .stoppable()
            .periodic_access(Duration::from_millis(5), move |src| {
                if controls.stopped.load(Ordering::SeqCst) {
                    src.stop();
                } else {
                    src.inner_mut().set_factor(*controls.volume.lock().unwrap());
                    src.inner_mut()
                        .inner_mut()
                        .set_paused(controls.pause.load(Ordering::SeqCst));
                }
            })
            .convert_samples();
        self.sound_count.fetch_add(1, Ordering::Relaxed);
        let source = Done::new(source, self.sound_count.clone());
        *self.sleep_until_end.lock().unwrap() = Some(self.queue_tx.append_with_signal(source));
    }
}

impl<S: Sample + Send + 'static> SinkControl for Sink<S> {
    #[inline]
    fn volume(&self) -> f32 {
        *self.controls.volume.lock().unwrap()
    }

    #[inline]
    fn set_volume(&self, value: f32) {
        *self.controls.volume.lock().unwrap() = value;
    }

    #[inline]
    fn play(&self) {
        self.controls.pause.store(false, Ordering::SeqCst);
    }

    fn pause(&self) {
        self.controls.pause.store(true, Ordering::SeqCst);
    }

    fn is_paused(&self) -> bool {
        self.controls.pause.load(Ordering::SeqCst)
    }

    #[inline]
    fn stop(&mut self) {
        self.controls.stopped.store(true, Ordering::SeqCst);
    }

    #[inline]
    fn detach(mut self) {
        self.detached = true;
    }

    #[inline]
    fn sleep_until_end(&self) {
        if let Some(sleep_until_end) = self.sleep_until_end.lock().unwrap().take() {
            let _ = sleep_until_end.recv();
        }
    }

    #[inline]
    fn empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    fn len(&self) -> usize {
        self.sound_count.load(Ordering::Relaxed)
    }
}

impl<S: Sample + Send + 'static> Drop for Sink<S> {
    #[inline]
    fn drop(&mut self) {
        self.queue_tx.set_keep_alive_if_empty(false);

        if !self.detached {
            self.controls.stopped.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::buffer::SamplesBuffer;
    use crate::{Sink, SinkAppender, SinkControl, Source};

    #[test]
    fn test_pause_and_stop() {
        let (mut sink, mut queue_rx) = Sink::new_idle();

        // assert_eq!(queue_rx.next(), Some(0.0));

        let v = vec![10i16, -10, 20, -20, 30, -30];

        // Low rate to ensure immediate control.
        sink.append(SamplesBuffer::new(1, 1, v.clone()));
        let mut src = SamplesBuffer::new(1, 1, v).convert_samples();

        assert_eq!(queue_rx.next(), src.next());
        assert_eq!(queue_rx.next(), src.next());

        sink.pause();

        assert_eq!(queue_rx.next(), Some(0.0));

        sink.play();

        assert_eq!(queue_rx.next(), src.next());
        assert_eq!(queue_rx.next(), src.next());

        sink.stop();

        assert_eq!(queue_rx.next(), Some(0.0));

        assert_eq!(sink.empty(), true);
    }

    #[test]
    fn test_volume() {
        let (sink, mut queue_rx) = Sink::new_idle();

        let v = vec![10i16, -10, 20, -20, 30, -30];

        // High rate to avoid immediate control.
        sink.append(SamplesBuffer::new(2, 44100, v.clone()));
        let src = SamplesBuffer::new(2, 44100, v.clone()).convert_samples();

        let mut src = src.amplify(0.5);
        sink.set_volume(0.5);

        for _ in 0..v.len() {
            assert_eq!(queue_rx.next(), src.next());
        }
    }
}
