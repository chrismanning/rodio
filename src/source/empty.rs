use std::marker::PhantomData;
use std::time::Duration;

use crate::{Sample, Source};
use std::mem::size_of;

/// An empty source.
#[derive(Debug, Copy, Clone)]
pub struct Empty<S> {
    channels: u16,
    sample_rate: u32,
    marker: PhantomData<S>,
}

impl<S> Default for Empty<S> {
    #[inline]
    fn default() -> Self {
        Self::new(1, 4800)
    }
}

impl<S> Empty<S> {
    #[inline]
    pub fn new(channels: u16, sample_rate: u32) -> Empty<S> {
        Empty {
            channels,
            sample_rate,
            marker: PhantomData,
        }
    }
}

impl<S> Iterator for Empty<S> {
    type Item = S;

    #[inline]
    fn next(&mut self) -> Option<S> {
        None
    }
}

impl<S> Source for Empty<S>
where
    S: Sample,
{
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    #[inline]
    fn channels(&self) -> u16 {
        self.channels
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::new(0, 0))
    }

    #[inline]
    fn bits_per_sample(&self) -> u8 {
        size_of::<S>() as u8
    }
}
