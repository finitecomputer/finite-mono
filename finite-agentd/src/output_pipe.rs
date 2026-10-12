//! Report-only observations of the output pipes `finite-agentd` inherited.
//!
//! In the Agent Runtime image agentd's stdout and stderr are the container
//! output streams, and every supervised child inherits them
//! (`supervisor::spawn_process`). Traced from kata-agent 3.29.0 source, not
//! observed in a booted guest: under Kata each one is an anonymous pipe that
//! kata-agent reads only when the host shim asks for more output.
//!
//! A dedicated thread duplicates both descriptors once and samples them every
//! [`SAMPLE_INTERVAL`]: `FIONREAD` (bytes waiting) and `F_GETPIPE_SZ`
//! (capacity). They are two separate reads, not one atomic snapshot, so a
//! concurrent resize can leave `buffered_bytes` above `capacity_bytes`.
//! `observed_at_ms` is the wall-clock time after both reads returned; if the
//! second read waited, the first value is older than that.
//!
//! `observed_decrease_bytes` is the saturating sum of drops in
//! `buffered_bytes` between consecutive successful samples, and nothing more.
//! It is not a count of bytes read. A writer in packet mode (`O_DIRECT`,
//! which can be switched on with `F_SETFL` after sampling started) writes
//! packets, and the kernel may split a large write across several. A read
//! shorter than the packet it reads returns part of it, the kernel discards
//! the rest of that packet, and the discarded bytes count as a drop. Output
//! written and removed between two samples does not show up at all.
//!
//! Nothing here is a verdict. A full pipe does not prove that a particular
//! writer is waiting (small writes can still merge into the last page), that
//! the reader stopped, or that a host collector is missing. A growing
//! decrease total does not prove that a collector received or delivered
//! anything, and an unchanged one does not prove that nothing was read. None
//! of it describes gateway, channel or reply progress, and readiness never
//! reads it.
//!
//! The kernel takes the pipe lock for both reads, and a writer holds that lock
//! while it copies, so sampling stays off the status heartbeat: the heartbeat
//! copies the latest published sample, and every sample carries its own
//! `observed_at_ms`. The thread never writes to stdout or stderr; they are the
//! pipes it observes and may be full.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

#[cfg(target_os = "linux")]
use crate::supervisor::now_ms;

pub(crate) const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// The latest sample of agentd's stdout and stderr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerOutputStatus {
    pub sample_interval_ms: u64,
    pub stdout: OutputStreamObservation,
    pub stderr: OutputStreamObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputStreamObservation {
    Pipe(PipeObservation),
    NotPipe {
        observed_at_ms: u64,
        file_type: OutputFileType,
    },
    /// stderr only: the same pipe object as stdout, which is sampled once.
    SamePipeAsStdout,
    Unavailable {
        observed_at_ms: u64,
        error: ObservationError,
    },
    UnsupportedPlatform,
    SamplerNotStarted,
}

/// Raw values from one sample, plus the drops in `buffered_bytes` summed
/// across the samples of this same pipe since `first_observed_at_ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipeObservation {
    /// Wall-clock time after both reads returned, not a capture instant.
    pub observed_at_ms: u64,
    /// `FIONREAD`: bytes waiting to be read at this sample.
    pub buffered_bytes: u64,
    /// `F_GETPIPE_SZ`, read separately from `buffered_bytes`; a concurrent
    /// resize can leave it below `buffered_bytes`.
    pub capacity_bytes: u64,
    pub first_observed_at_ms: u64,
    pub sample_count: u64,
    /// Saturating sum of drops in `buffered_bytes` between consecutive
    /// successful samples. Not bytes read, received or delivered (packet-mode
    /// discards count too), and not a rate.
    pub observed_decrease_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFileType {
    RegularFile,
    CharacterDevice,
    BlockDevice,
    Socket,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObservationError {
    #[serde(rename = "EBADF")]
    BadDescriptor,
    #[serde(rename = "EINVAL")]
    InvalidArgument,
    #[serde(rename = "ENOTTY")]
    NotSupported,
    #[serde(rename = "ENOMEM")]
    OutOfMemory,
    #[serde(rename = "EMFILE")]
    ProcessDescriptorLimit,
    #[serde(rename = "ENFILE")]
    SystemDescriptorLimit,
    #[serde(rename = "EINTR")]
    Interrupted,
    #[serde(rename = "EIO")]
    InputOutput,
    #[serde(rename = "other")]
    Other,
}

impl From<rustix::io::Errno> for ObservationError {
    fn from(errno: rustix::io::Errno) -> Self {
        use rustix::io::Errno;
        match errno {
            Errno::BADF => Self::BadDescriptor,
            Errno::INVAL => Self::InvalidArgument,
            Errno::NOTTY => Self::NotSupported,
            Errno::NOMEM => Self::OutOfMemory,
            Errno::MFILE => Self::ProcessDescriptorLimit,
            Errno::NFILE => Self::SystemDescriptorLimit,
            Errno::INTR => Self::Interrupted,
            Errno::IO => Self::InputOutput,
            _ => Self::Other,
        }
    }
}

impl From<std::io::Error> for ObservationError {
    fn from(error: std::io::Error) -> Self {
        error.raw_os_error().map_or(Self::Other, |code| {
            rustix::io::Errno::from_raw_os_error(code).into()
        })
    }
}

/// Per-pipe state carried between samples. Only Linux builds sample; other
/// platforms still compile and test the pure parts.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PipeTrack {
    first_observed_at_ms: u64,
    sample_count: u64,
    last_buffered_bytes: u64,
    observed_decrease_bytes: u64,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl PipeTrack {
    /// Adds the drop, if any, since the previous successful sample; a rise
    /// adds nothing. A failed sample leaves the track as it was, so the next
    /// successful sample is compared with the last successful one.
    fn advance(previous: Option<Self>, buffered_bytes: u64, observed_at_ms: u64) -> Self {
        let Some(previous) = previous else {
            return Self {
                first_observed_at_ms: observed_at_ms,
                sample_count: 1,
                last_buffered_bytes: buffered_bytes,
                observed_decrease_bytes: 0,
            };
        };
        Self {
            first_observed_at_ms: previous.first_observed_at_ms,
            sample_count: previous.sample_count.saturating_add(1),
            last_buffered_bytes: buffered_bytes,
            observed_decrease_bytes: previous
                .observed_decrease_bytes
                .saturating_add(previous.last_buffered_bytes.saturating_sub(buffered_bytes)),
        }
    }

    fn observation(self, capacity_bytes: u64, observed_at_ms: u64) -> PipeObservation {
        PipeObservation {
            observed_at_ms,
            buffered_bytes: self.last_buffered_bytes,
            capacity_bytes,
            first_observed_at_ms: self.first_observed_at_ms,
            sample_count: self.sample_count,
            observed_decrease_bytes: self.observed_decrease_bytes,
        }
    }
}

/// Read side of the sampler. Reading never waits for a sample.
#[derive(Clone)]
pub(crate) struct OutputObserver {
    latest: watch::Receiver<Option<ContainerOutputStatus>>,
}

impl OutputObserver {
    /// Observe this process's own stdout and stderr.
    pub(crate) fn start() -> Self {
        use std::os::fd::AsFd;
        observe_descriptors(
            std::io::stdout().as_fd(),
            std::io::stderr().as_fd(),
            SAMPLE_INTERVAL,
        )
    }

    pub(crate) fn latest(&self) -> Option<ContainerOutputStatus> {
        self.latest.borrow().clone()
    }

    fn fixed(status: ContainerOutputStatus) -> Self {
        let (_sender, latest) = watch::channel(Some(status));
        Self { latest }
    }
}

fn interval_ms(interval: Duration) -> u64 {
    u64::try_from(interval.as_millis()).unwrap_or(u64::MAX)
}

/// Run `sample` on its own thread every `interval` and publish each result.
/// The thread stops once every [`OutputObserver`] is dropped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn spawn_sampler<F>(interval: Duration, mut sample: F) -> OutputObserver
where
    F: FnMut() -> ContainerOutputStatus + Send + 'static,
{
    let (sender, latest) = watch::channel(None);
    let spawned = std::thread::Builder::new()
        .name("agentd-output-observer".to_owned())
        .spawn(move || {
            loop {
                // Sample before publishing: the watch lock is never held while
                // the kernel may make this thread wait for the pipe lock.
                let status = sample();
                sender.send_replace(Some(status));
                std::thread::sleep(interval);
                if sender.is_closed() {
                    return;
                }
            }
        });
    match spawned {
        Ok(_) => OutputObserver { latest },
        Err(_) => OutputObserver::fixed(ContainerOutputStatus {
            sample_interval_ms: interval_ms(interval),
            stdout: OutputStreamObservation::SamplerNotStarted,
            stderr: OutputStreamObservation::SamplerNotStarted,
        }),
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn observe_descriptors(
    _stdout: std::os::fd::BorrowedFd<'_>,
    _stderr: std::os::fd::BorrowedFd<'_>,
    interval: Duration,
) -> OutputObserver {
    OutputObserver::fixed(ContainerOutputStatus {
        sample_interval_ms: interval_ms(interval),
        stdout: OutputStreamObservation::UnsupportedPlatform,
        stderr: OutputStreamObservation::UnsupportedPlatform,
    })
}

/// Duplicate both descriptors (close-on-exec, so supervised children do not
/// inherit the copies) and sample them on a dedicated thread. The copies pin
/// the observed pipe objects for the life of the sampler.
#[cfg(target_os = "linux")]
pub(crate) fn observe_descriptors(
    stdout: std::os::fd::BorrowedFd<'_>,
    stderr: std::os::fd::BorrowedFd<'_>,
    interval: Duration,
) -> OutputObserver {
    let stdout = StreamTarget::duplicate(stdout);
    let mut stderr = StreamTarget::duplicate(stderr);
    if stdout.same_pipe_as(&stderr) {
        stderr = StreamTarget::SamePipeAsStdout;
    }
    let mut stdout = StreamSampler::new(stdout);
    let mut stderr = StreamSampler::new(stderr);
    let sample_interval_ms = interval_ms(interval);
    spawn_sampler(interval, move || ContainerOutputStatus {
        sample_interval_ms,
        stdout: stdout.sample(),
        stderr: stderr.sample(),
    })
}

#[cfg(target_os = "linux")]
enum StreamTarget {
    Descriptor(std::fs::File),
    SamePipeAsStdout,
    Unavailable(ObservationError),
}

#[cfg(target_os = "linux")]
impl StreamTarget {
    fn duplicate(fd: std::os::fd::BorrowedFd<'_>) -> Self {
        match fd.try_clone_to_owned() {
            Ok(owned) => Self::Descriptor(owned.into()),
            Err(error) => Self::Unavailable(error.into()),
        }
    }

    fn same_pipe_as(&self, other: &Self) -> bool {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let (Self::Descriptor(left), Self::Descriptor(right)) = (self, other) else {
            return false;
        };
        match (left.metadata(), right.metadata()) {
            (Ok(left), Ok(right)) => {
                left.file_type().is_fifo() && left.dev() == right.dev() && left.ino() == right.ino()
            }
            _ => false,
        }
    }
}

#[cfg(target_os = "linux")]
enum RawSample {
    Pipe {
        buffered_bytes: u64,
        capacity_bytes: u64,
    },
    NotPipe(OutputFileType),
}

#[cfg(target_os = "linux")]
fn probe(file: &std::fs::File) -> Result<RawSample, ObservationError> {
    use std::os::unix::fs::FileTypeExt;
    let file_type = file.metadata()?.file_type();
    if !file_type.is_fifo() {
        let kind = if file_type.is_file() {
            OutputFileType::RegularFile
        } else if file_type.is_char_device() {
            OutputFileType::CharacterDevice
        } else if file_type.is_block_device() {
            OutputFileType::BlockDevice
        } else if file_type.is_socket() {
            OutputFileType::Socket
        } else if file_type.is_dir() {
            OutputFileType::Directory
        } else if file_type.is_symlink() {
            OutputFileType::Symlink
        } else {
            OutputFileType::Other
        };
        return Ok(RawSample::NotPipe(kind));
    }
    let buffered_bytes = rustix::io::ioctl_fionread(file)?;
    let capacity_bytes = rustix::pipe::fcntl_getpipe_size(file)?;
    Ok(RawSample::Pipe {
        buffered_bytes,
        capacity_bytes: u64::try_from(capacity_bytes).unwrap_or(u64::MAX),
    })
}

#[cfg(target_os = "linux")]
struct StreamSampler {
    target: StreamTarget,
    track: Option<PipeTrack>,
}

#[cfg(target_os = "linux")]
impl StreamSampler {
    fn new(target: StreamTarget) -> Self {
        Self {
            target,
            track: None,
        }
    }

    fn sample(&mut self) -> OutputStreamObservation {
        let file = match &self.target {
            StreamTarget::Descriptor(file) => file,
            StreamTarget::SamePipeAsStdout => return OutputStreamObservation::SamePipeAsStdout,
            StreamTarget::Unavailable(error) => {
                return OutputStreamObservation::Unavailable {
                    observed_at_ms: now_ms(),
                    error: *error,
                };
            }
        };
        let sampled = probe(file);
        let observed_at_ms = now_ms();
        match sampled {
            Ok(RawSample::Pipe {
                buffered_bytes,
                capacity_bytes,
            }) => {
                let track = PipeTrack::advance(self.track, buffered_bytes, observed_at_ms);
                self.track = Some(track);
                OutputStreamObservation::Pipe(track.observation(capacity_bytes, observed_at_ms))
            }
            Ok(RawSample::NotPipe(file_type)) => OutputStreamObservation::NotPipe {
                observed_at_ms,
                file_type,
            },
            Err(error) => OutputStreamObservation::Unavailable {
                observed_at_ms,
                error,
            },
        }
    }
}

#[cfg(test)]
mod seam_tests;

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use super::*;

    #[test]
    fn drops_between_samples_accumulate_and_rises_do_not_count() {
        let mut track = None;
        for (at_ms, buffered) in [(10, 0), (20, 100), (30, 40), (40, 40), (50, 1000), (60, 0)] {
            track = Some(PipeTrack::advance(track, buffered, at_ms));
        }
        let observation = track.unwrap().observation(65_536, 60);
        assert_eq!(
            observation,
            PipeObservation {
                observed_at_ms: 60,
                buffered_bytes: 0,
                capacity_bytes: 65_536,
                first_observed_at_ms: 10,
                sample_count: 6,
                // 100 -> 40 and 1000 -> 0; the rises in between add nothing.
                observed_decrease_bytes: 60 + 1000,
            }
        );
    }

    #[test]
    fn a_steady_or_rising_pipe_records_no_decrease() {
        let mut track = None;
        for (at_ms, buffered) in [(1, 65_536), (2, 65_536), (3, 65_536), (4, 70_000)] {
            track = Some(PipeTrack::advance(track, buffered, at_ms));
        }
        assert_eq!(track.unwrap().observed_decrease_bytes, 0);
        assert_eq!(track.unwrap().sample_count, 4);
    }

    #[test]
    fn counters_saturate_instead_of_panicking() {
        let track = PipeTrack {
            first_observed_at_ms: 0,
            sample_count: u64::MAX,
            last_buffered_bytes: u64::MAX,
            observed_decrease_bytes: u64::MAX - 1,
        };
        let next = PipeTrack::advance(Some(track), 0, 1);
        assert_eq!(next.sample_count, u64::MAX);
        assert_eq!(next.observed_decrease_bytes, u64::MAX);
    }

    #[test]
    fn errno_names_come_from_a_fixed_vocabulary() {
        use rustix::io::Errno;
        assert_eq!(
            serde_json::to_value(ObservationError::from(Errno::BADF)).unwrap(),
            "EBADF"
        );
        assert_eq!(
            ObservationError::from(std::io::Error::from_raw_os_error(
                Errno::MFILE.raw_os_error()
            )),
            ObservationError::ProcessDescriptorLimit
        );
        assert_eq!(
            ObservationError::from(Errno::ACCESS),
            ObservationError::Other
        );
        assert_eq!(
            ObservationError::from(std::io::Error::other("no errno")),
            ObservationError::Other
        );
    }

    #[test]
    fn the_wire_shape_is_tagged_by_kind() {
        let status = ContainerOutputStatus {
            sample_interval_ms: 1000,
            stdout: OutputStreamObservation::Pipe(PipeObservation {
                observed_at_ms: 5,
                buffered_bytes: 4096,
                capacity_bytes: 65_536,
                first_observed_at_ms: 1,
                sample_count: 3,
                observed_decrease_bytes: 7,
            }),
            stderr: OutputStreamObservation::NotPipe {
                observed_at_ms: 5,
                file_type: OutputFileType::CharacterDevice,
            },
        };
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(
            json,
            r#"{"sample_interval_ms":1000,"stdout":{"kind":"pipe","observed_at_ms":5,"buffered_bytes":4096,"capacity_bytes":65536,"first_observed_at_ms":1,"sample_count":3,"observed_decrease_bytes":7},"stderr":{"kind":"not_pipe","observed_at_ms":5,"file_type":"character_device"}}"#
        );
        assert_eq!(
            serde_json::from_str::<ContainerOutputStatus>(&json).unwrap(),
            status
        );
        for (observation, json) in [
            (
                OutputStreamObservation::SamePipeAsStdout,
                r#"{"kind":"same_pipe_as_stdout"}"#,
            ),
            (
                OutputStreamObservation::UnsupportedPlatform,
                r#"{"kind":"unsupported_platform"}"#,
            ),
            (
                OutputStreamObservation::SamplerNotStarted,
                r#"{"kind":"sampler_not_started"}"#,
            ),
            (
                OutputStreamObservation::Unavailable {
                    observed_at_ms: 9,
                    error: ObservationError::BadDescriptor,
                },
                r#"{"kind":"unavailable","observed_at_ms":9,"error":"EBADF"}"#,
            ),
        ] {
            assert_eq!(serde_json::to_string(&observation).unwrap(), json);
        }
    }

    fn sample_at(observed_at_ms: u64) -> ContainerOutputStatus {
        ContainerOutputStatus {
            sample_interval_ms: 10,
            stdout: OutputStreamObservation::Unavailable {
                observed_at_ms,
                error: ObservationError::Other,
            },
            stderr: OutputStreamObservation::SamePipeAsStdout,
        }
    }

    pub(super) fn wait_for<T>(timeout: Duration, mut ready: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(value) = ready() {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "condition not met within {timeout:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_blocked_sample_never_blocks_readers_and_stays_visibly_old() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (entered_tx, entered_rx) = mpsc::channel::<u64>();
        let mut calls = 0u64;
        let observer = spawn_sampler(Duration::from_millis(10), move || {
            calls += 1;
            let _ = entered_tx.send(calls);
            if calls == 2 {
                // A probe double stuck the way a pipe-lock wait could be.
                let _ = release_rx.recv_timeout(Duration::from_secs(30));
            }
            sample_at(calls)
        });
        assert_eq!(entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        assert_eq!(entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2);

        let started = Instant::now();
        for _ in 0..50 {
            assert_eq!(observer.latest(), Some(sample_at(1)));
        }
        assert!(started.elapsed() < Duration::from_secs(1));

        release_tx.send(()).unwrap();
        wait_for(Duration::from_secs(5), || {
            observer
                .latest()
                .filter(|latest| *latest != sample_at(1))
                .map(|_| ())
        });
    }

    #[test]
    fn the_sampler_thread_stops_after_the_last_reader_is_dropped() {
        let (tick_tx, tick_rx) = mpsc::channel::<()>();
        let observer = spawn_sampler(Duration::from_millis(5), move || {
            let _ = tick_tx.send(());
            sample_at(0)
        });
        tick_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(observer);
        // The thread owns the only sender; once it exits, recv disconnects.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match tick_rx.recv_timeout(Duration::from_millis(100)) {
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                _ => assert!(Instant::now() < deadline, "sampler thread kept running"),
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn other_platforms_report_unsupported_instead_of_sampling() {
        use std::os::fd::AsFd;
        let (reader, writer) = std::io::pipe().unwrap();
        let observer =
            observe_descriptors(writer.as_fd(), reader.as_fd(), Duration::from_millis(5));
        assert_eq!(
            observer.latest(),
            Some(ContainerOutputStatus {
                sample_interval_ms: 5,
                stdout: OutputStreamObservation::UnsupportedPlatform,
                stderr: OutputStreamObservation::UnsupportedPlatform,
            })
        );
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use std::io::{Read, Write};
        use std::os::fd::{AsFd, BorrowedFd};
        use std::os::unix::net::UnixStream;

        use rustix::pipe::PipeFlags;

        use super::super::*;
        use super::wait_for;

        const VERDICT_WORDS: [&str; 7] = [
            "stall",
            "blocked",
            "healthy",
            "degraded",
            "collector",
            "dead",
            "missing",
        ];

        fn sampler(fd: BorrowedFd<'_>) -> StreamSampler {
            StreamSampler::new(StreamTarget::duplicate(fd))
        }

        fn pipe_sample(sampler: &mut StreamSampler) -> PipeObservation {
            match sampler.sample() {
                OutputStreamObservation::Pipe(observation) => observation,
                other => panic!("expected a pipe observation, got {other:?}"),
            }
        }

        fn nonblocking_pipe() -> (std::os::fd::OwnedFd, std::os::fd::OwnedFd) {
            rustix::pipe::pipe_with(PipeFlags::CLOEXEC | PipeFlags::NONBLOCK).unwrap()
        }

        /// Write `chunk`-byte writes until the kernel refuses one; returns the
        /// bytes it accepted.
        fn fill(writer: &std::os::fd::OwnedFd, chunk: usize) -> u64 {
            let bytes = vec![b'x'; chunk];
            let mut accepted = 0u64;
            loop {
                match rustix::io::write(writer, &bytes) {
                    Ok(written) => accepted += written as u64,
                    Err(rustix::io::Errno::AGAIN) => return accepted,
                    Err(error) => panic!("unexpected write error {error}"),
                }
            }
        }

        #[test]
        fn an_unread_pipe_reports_exactly_the_bytes_the_kernel_accepted() {
            let (_reader, writer) = nonblocking_pipe();
            let mut sampler = sampler(writer.as_fd());
            let page = rustix::param::page_size();

            let accepted = fill(&writer, page);
            let full = pipe_sample(&mut sampler);
            assert_eq!(full.buffered_bytes, accepted);
            // Page-sized writes fill every slot completely, so here the bytes
            // the kernel accepted are the whole capacity.
            assert_eq!(full.capacity_bytes, accepted);
            assert_eq!(full.observed_decrease_bytes, 0);
            assert!(full.observed_at_ms > 0);
        }

        #[test]
        fn a_refused_write_does_not_mean_the_pipe_is_byte_full_or_closed_to_all_writes() {
            let (_reader, writer) = nonblocking_pipe();
            let mut sampler = sampler(writer.as_fd());
            let page = rustix::param::page_size();
            let chunk = page / 4 + 1;
            let per_page = (page / chunk) * chunk;

            // Writes that cannot merge into the last page take a new slot; the
            // kernel refuses one once every slot is used, short of capacity.
            let accepted = fill(&writer, chunk);
            let refused = pipe_sample(&mut sampler);
            assert_eq!(refused.buffered_bytes, accepted);
            assert!(refused.buffered_bytes < refused.capacity_bytes);
            assert_eq!(
                refused.buffered_bytes,
                (refused.capacity_bytes / page as u64) * per_page as u64
            );
            // A smaller write still merges into the last page of that pipe.
            let room = page - per_page;
            assert_eq!(rustix::io::write(&writer, &vec![b'y'; room]).unwrap(), room);
            assert_eq!(
                pipe_sample(&mut sampler).buffered_bytes,
                accepted + room as u64
            );
        }

        #[test]
        fn reads_between_samples_show_up_as_a_decrease() {
            let (reader, writer) = nonblocking_pipe();
            let mut sampler = sampler(writer.as_fd());
            let accepted = fill(&writer, rustix::param::page_size());
            let before = pipe_sample(&mut sampler);

            let mut buffer = vec![0u8; 10_000];
            assert_eq!(rustix::io::read(&reader, &mut buffer).unwrap(), 10_000);
            let after = pipe_sample(&mut sampler);
            assert_eq!(after.buffered_bytes, accepted - 10_000);
            assert_eq!(after.observed_decrease_bytes, 10_000);
            assert_eq!(after.first_observed_at_ms, before.first_observed_at_ms);
            assert_eq!(after.sample_count, 2);

            // Refilling is not reading: the decrease total stays put.
            fill(&writer, 1);
            let refilled = pipe_sample(&mut sampler);
            assert_eq!(refilled.observed_decrease_bytes, 10_000);
            let mut drain = vec![0u8; accepted as usize];
            let mut drained = 0;
            while let Ok(read) = rustix::io::read(&reader, &mut drain) {
                drained += read as u64;
            }
            let empty = pipe_sample(&mut sampler);
            assert_eq!(empty.buffered_bytes, 0);
            assert_eq!(empty.observed_decrease_bytes, 10_000 + drained);
        }

        /// Write 4 bytes, read 1, and return the bytes that read returned,
        /// the decrease the sampler recorded across it, and the bytes left.
        fn write_four_read_one(
            sampler: &mut StreamSampler,
            writer: &std::os::fd::OwnedFd,
            reader: &std::os::fd::OwnedFd,
        ) -> (usize, u64, usize) {
            assert_eq!(rustix::io::write(writer, b"abcd").unwrap(), 4);
            let before = pipe_sample(sampler);
            assert_eq!(before.buffered_bytes, 4);
            let returned = rustix::io::read(reader, &mut [0u8; 1]).unwrap();
            let after = pipe_sample(sampler);
            let left = match rustix::io::read(reader, &mut [0u8; 16]) {
                Ok(left) => left,
                Err(rustix::io::Errno::AGAIN) => 0,
                Err(error) => panic!("unexpected read error {error}"),
            };
            let decrease = after.observed_decrease_bytes - before.observed_decrease_bytes;
            (returned, decrease, left)
        }

        #[test]
        fn a_short_read_of_a_packet_counts_the_discarded_rest_as_a_decrease() {
            // A byte-stream pipe keeps what a short read leaves behind.
            let (reader, writer) = nonblocking_pipe();
            let mut stream = sampler(writer.as_fd());
            assert_eq!(
                write_four_read_one(&mut stream, &writer, &reader),
                (1, 1, 3)
            );

            // In packet mode this four-byte write is one packet. The kernel
            // discards the part a short read did not return, so the decrease
            // exceeds the bytes any reader got: it is not a count of bytes read.
            let (reader, writer) = rustix::pipe::pipe_with(
                PipeFlags::CLOEXEC | PipeFlags::NONBLOCK | PipeFlags::DIRECT,
            )
            .unwrap();
            let mut packets = sampler(writer.as_fd());
            assert_eq!(
                write_four_read_one(&mut packets, &writer, &reader),
                (1, 4, 0)
            );

            // A writer can switch an ordinary pipe to packet mode after the
            // observer started, as a child could on its inherited stdout.
            let (reader, writer) = nonblocking_pipe();
            let mut switched = sampler(writer.as_fd());
            let flags = rustix::fs::fcntl_getfl(&writer).unwrap();
            rustix::fs::fcntl_setfl(&writer, flags | rustix::fs::OFlags::DIRECT).unwrap();
            assert_eq!(
                write_four_read_one(&mut switched, &writer, &reader),
                (1, 4, 0)
            );
        }

        #[test]
        fn an_idle_pipe_stays_empty_without_any_decrease() {
            let (_reader, writer) = std::io::pipe().unwrap();
            let mut sampler = sampler(writer.as_fd());
            for count in 1..=5 {
                let idle = pipe_sample(&mut sampler);
                assert_eq!(idle.buffered_bytes, 0);
                assert!(idle.capacity_bytes > 0);
                assert_eq!(idle.observed_decrease_bytes, 0);
                assert_eq!(idle.sample_count, count);
            }
        }

        #[test]
        fn files_devices_and_sockets_are_reported_as_not_pipes() {
            let file = tempfile::tempfile().unwrap();
            let null = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/null")
                .unwrap();
            let (socket, _peer) = UnixStream::pair().unwrap();
            for (fd, expected) in [
                (file.as_fd(), OutputFileType::RegularFile),
                (null.as_fd(), OutputFileType::CharacterDevice),
                (socket.as_fd(), OutputFileType::Socket),
            ] {
                match sampler(fd).sample() {
                    OutputStreamObservation::NotPipe { file_type, .. } => {
                        assert_eq!(file_type, expected);
                    }
                    other => panic!("expected not_pipe, got {other:?}"),
                }
            }
        }

        #[test]
        fn one_pipe_on_both_streams_is_sampled_once() {
            let (reader, writer) = std::io::pipe().unwrap();
            let shared = observe_descriptors(writer.as_fd(), writer.as_fd(), SAMPLE_INTERVAL);
            let status = wait_for(Duration::from_secs(5), || shared.latest());
            assert!(matches!(status.stdout, OutputStreamObservation::Pipe(_)));
            assert_eq!(status.stderr, OutputStreamObservation::SamePipeAsStdout);

            // Both ends of one pipe are the same object too.
            let ends = observe_descriptors(writer.as_fd(), reader.as_fd(), SAMPLE_INTERVAL);
            let status = wait_for(Duration::from_secs(5), || ends.latest());
            assert_eq!(status.stderr, OutputStreamObservation::SamePipeAsStdout);

            let (_other_reader, other_writer) = std::io::pipe().unwrap();
            let separate =
                observe_descriptors(writer.as_fd(), other_writer.as_fd(), SAMPLE_INTERVAL);
            let status = wait_for(Duration::from_secs(5), || separate.latest());
            assert!(matches!(status.stdout, OutputStreamObservation::Pipe(_)));
            assert!(matches!(status.stderr, OutputStreamObservation::Pipe(_)));
        }

        #[test]
        fn a_slow_reader_draining_one_large_write_gets_raw_evidence_and_no_verdict() {
            let (mut reader, mut writer) = std::io::pipe().unwrap();
            let observer =
                observe_descriptors(writer.as_fd(), writer.as_fd(), Duration::from_millis(20));
            const TOTAL: usize = 1 << 20;

            let (written_tx, written_rx) = std::sync::mpsc::channel();
            let writer_thread = std::thread::spawn(move || {
                // One large blocking write: the kernel keeps it pending while
                // the pipe is full and resumes it each time the reader frees a
                // page. `write_all` only matters if a signal cuts it short.
                let written = writer.write_all(&vec![b'z'; TOTAL]);
                let _ = written_tx.send(written.map_err(|error| error.to_string()));
            });
            let reader_thread = std::thread::spawn(move || {
                let mut buffer = [0u8; 4096];
                let mut total = 0usize;
                while total < TOTAL {
                    std::thread::sleep(Duration::from_millis(4));
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => total += read,
                    }
                }
                total
            });

            let mut while_pending = Vec::new();
            let written = loop {
                if let Ok(written) = written_rx.try_recv() {
                    break written;
                }
                if let Some(status) = observer.latest() {
                    while_pending.push(status);
                }
                assert!(while_pending.len() < 10_000, "the write never completed");
                std::thread::sleep(Duration::from_millis(5));
            };
            writer_thread.join().unwrap();
            // The sampler's copies keep the write end open; once they close,
            // the reader sees EOF even if the write fell short.
            drop(observer);
            let read = reader_thread.join().unwrap();
            assert_eq!(written, Ok(()));
            assert_eq!(read, TOTAL);

            let pipe_samples: Vec<PipeObservation> = while_pending
                .iter()
                .filter_map(|status| match &status.stdout {
                    OutputStreamObservation::Pipe(observation) => Some(observation.clone()),
                    _ => None,
                })
                .collect();
            assert!(pipe_samples.iter().any(|sample| sample.buffered_bytes > 0));
            let last = pipe_samples.last().unwrap();
            assert!(last.observed_decrease_bytes <= TOTAL as u64);
            let serialized = serde_json::to_string(&while_pending).unwrap();
            for word in VERDICT_WORDS {
                assert!(
                    !serialized.contains(word),
                    "verdict word {word:?} published"
                );
            }
        }
    }
}
