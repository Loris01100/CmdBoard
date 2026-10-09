//! Optimization screen: a few benchmarks of the CPU, memory and system disk, and the
//! Windows gaming settings, read and written in the registry.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::ptr::null_mut;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_DWORD, RRF_RT_REG_DWORD, RegGetValueW,
    RegSetKeyValueW,
};

use crate::launcher::programs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bench {
    CpuSingle,
    CpuMulti,
    Memory,
    Disk,
}

impl Bench {
    pub const ALL: [Bench; 4] = [
        Bench::CpuSingle,
        Bench::CpuMulti,
        Bench::Memory,
        Bench::Disk,
    ];

    pub fn label(self) -> String {
        match self {
            Bench::CpuSingle => t!("optimize.cpu_single"),
            Bench::CpuMulti => t!("optimize.cpu_multi"),
            Bench::Memory => t!("optimize.memory"),
            Bench::Disk => t!("optimize.disk"),
        }
    }
}

/// What a benchmark measured, per second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Score {
    /// Millions of operations.
    Ops(f64),
    Bytes(f64),
    Disk {
        write: f64,
        read: f64,
    },
}

/// Just what tells the machine apart, for reading the scores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct System {
    pub cpu: String,
    pub cores: usize,
    pub threads: usize,
    pub ram: u64,
}

pub fn system() -> System {
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();
    System {
        cpu: sys
            .cpus()
            .first()
            .map_or(String::new(), |c| c.brand().trim().into()),
        threads: sys.cpus().len(),
        cores: sysinfo::System::physical_core_count().unwrap_or(0),
        ram: sys.total_memory(),
    }
}

/// Runs a benchmark; `heavy` runs longer on more data. Blocks: call from a thread.
pub fn run(bench: Bench, heavy: bool) -> Result<Score, String> {
    let secs = Duration::from_secs(if heavy { 15 } else { 3 });
    match bench {
        Bench::CpuSingle => Ok(Score::Ops(cpu(1, secs))),
        Bench::CpuMulti => {
            let threads = thread::available_parallelism().map_or(1, std::num::NonZero::get);
            Ok(Score::Ops(cpu(threads, secs)))
        }
        Bench::Memory => memory(if heavy { 1 << 30 } else { 256 << 20 }, secs),
        Bench::Disk => disk(if heavy { 2 << 30 } else { 256 << 20 }),
    }
}

/// Integer work on `threads` threads for `duration`: millions of xorshift steps per second.
fn cpu(threads: usize, duration: Duration) -> f64 {
    const STEPS: u64 = 1 << 16;
    let start = Instant::now();
    let steps: u64 = thread::scope(|s| {
        let workers: Vec<_> = (0..threads as u64)
            .map(|seed| {
                s.spawn(move || {
                    let mut x = seed + 0x9E37_79B9_7F4A_7C15;
                    let mut done = 0;
                    // At least one batch, even if the thread started after `duration`.
                    loop {
                        for _ in 0..STEPS {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                        }
                        std::hint::black_box(x);
                        done += STEPS;
                        if start.elapsed() >= duration {
                            break done;
                        }
                    }
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap_or(0)).sum()
    });
    steps as f64 / start.elapsed().as_secs_f64() / 1e6
}

/// Copies a `size` buffer into another for `duration`: bytes copied per second.
fn memory(size: usize, duration: Duration) -> Result<Score, String> {
    let alloc = |fill: u8| -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        buf.try_reserve_exact(size)
            .map_err(|_| t!("optimize.no_memory"))?;
        buf.resize(size, fill);
        Ok(buf)
    };
    let src = alloc(0x5A)?;
    let mut dst = alloc(0)?;
    let start = Instant::now();
    let mut copied = 0u64;
    while start.elapsed() < duration {
        dst.copy_from_slice(std::hint::black_box(&src));
        copied += size as u64;
    }
    std::hint::black_box(&dst);
    Ok(Score::Bytes(copied as f64 / start.elapsed().as_secs_f64()))
}

/// Writes then reads back a `size` file in the temp folder, bypassing the Windows cache.
fn disk(size: u64) -> Result<Score, String> {
    const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
    const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
    const CHUNK: usize = 4 << 20;
    let path = std::env::temp_dir().join("cmdboard-bench.tmp");
    let free = programs::disks()
        .into_iter()
        .find(|d| path.to_string_lossy().to_uppercase().starts_with(d.letter))
        .map_or(0, |d| d.free);
    if free < size * 2 {
        return Err(t!("optimize.no_space"));
    }
    // Unbuffered I/O needs sector-aligned buffers.
    let mut raw = vec![0xA5u8; CHUNK + 4096];
    let offset = raw.as_ptr().align_offset(4096);
    let buf = &mut raw[offset..offset + CHUNK];
    let chunks = size / CHUNK as u64;

    let result = (|| -> std::io::Result<Score> {
        let start = Instant::now();
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH)
            .open(&path)?;
        for _ in 0..chunks {
            file.write_all(buf)?;
        }
        file.sync_all()?;
        let write = size as f64 / start.elapsed().as_secs_f64();
        drop(file);

        let start = Instant::now();
        let mut file: File = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_NO_BUFFERING)
            .open(&path)?;
        for _ in 0..chunks {
            file.read_exact(buf)?;
        }
        let read = size as f64 / start.elapsed().as_secs_f64();
        Ok(Score::Disk { write, read })
    })();
    let _ = fs::remove_file(&path);
    result.map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gaming {
    /// Windows prioritizes the game and pauses updates while playing.
    GameMode,
    /// Game Bar background recording (Game DVR), which costs some performance.
    Recording,
    /// Hardware-accelerated GPU scheduling: machine-wide, needs admin and a restart.
    GpuScheduling,
}

const GAME_BAR: &str = r"Software\Microsoft\GameBar";
const GAME_CONFIG: &str = r"System\GameConfigStore";
const GAME_DVR: &str = r"Software\Microsoft\Windows\CurrentVersion\GameDVR";
const GRAPHICS: &str = r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers";

impl Gaming {
    pub const ALL: [Gaming; 3] = [Gaming::GameMode, Gaming::Recording, Gaming::GpuScheduling];

    pub fn label(self) -> String {
        match self {
            Gaming::GameMode => t!("optimize.game_mode"),
            Gaming::Recording => t!("optimize.recording"),
            Gaming::GpuScheduling => t!("optimize.gpu_scheduling"),
        }
    }

    /// On or off; missing values are Windows' default.
    pub fn enabled(self) -> bool {
        match self {
            Gaming::GameMode => {
                dword(HKEY_CURRENT_USER, GAME_BAR, "AutoGameModeEnabled") != Some(0)
            }
            Gaming::Recording => {
                dword(HKEY_CURRENT_USER, GAME_CONFIG, "GameDVR_Enabled") != Some(0)
            }
            Gaming::GpuScheduling => dword(HKEY_LOCAL_MACHINE, GRAPHICS, "HwSchMode") == Some(2),
        }
    }

    /// Only the per-user settings are switched here; the others open their settings page.
    pub fn switchable(self) -> bool {
        self != Gaming::GpuScheduling
    }

    pub fn set(self, on: bool) -> anyhow::Result<()> {
        let on = u32::from(on);
        match self {
            Gaming::GameMode => set_dword(GAME_BAR, "AutoGameModeEnabled", on),
            Gaming::Recording => {
                set_dword(GAME_CONFIG, "GameDVR_Enabled", on)?;
                set_dword(GAME_DVR, "AppCaptureEnabled", on)
            }
            Gaming::GpuScheduling => anyhow::bail!("not switchable"),
        }
    }

    /// Its page in the Windows settings.
    pub fn page(self) -> &'static str {
        match self {
            Gaming::GameMode => "ms-settings:gaming-gamemode",
            Gaming::Recording => "ms-settings:gaming-gamedvr",
            Gaming::GpuScheduling => "ms-settings:display-advancedgraphics",
        }
    }
}

fn dword(root: HKEY, path: &str, name: &str) -> Option<u32> {
    let (path, name) = (programs::wide(path), programs::wide(name));
    let mut value = 0u32;
    let mut size = 4u32;
    // SAFETY: both strings are nul-terminated; `value` holds the 4 bytes of a DWORD.
    let status = unsafe {
        RegGetValueW(
            root,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&raw mut value).cast(),
            &raw mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}

/// Writes a DWORD under HKCU, creating the key if needed.
fn set_dword(path: &str, name: &str, value: u32) -> anyhow::Result<()> {
    let (path, name) = (programs::wide(path), programs::wide(name));
    // SAFETY: both strings are nul-terminated; `value` holds the 4 bytes of a DWORD.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            name.as_ptr(),
            REG_DWORD,
            (&raw const value).cast(),
            4,
        )
    };
    anyhow::ensure!(status == ERROR_SUCCESS, "registry error {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_benchmark_writes_and_reads_back() {
        let Ok(Score::Disk { write, read }) = disk(8 << 20) else {
            panic!("disk benchmark failed");
        };
        assert!(write > 0.0 && read > 0.0);
        assert!(!std::env::temp_dir().join("cmdboard-bench.tmp").exists());
        assert!(disk(u64::MAX / 4).is_err()); // never that much free space
    }

    #[test]
    fn gaming_settings_describe_themselves() {
        use std::collections::HashSet;
        let labels: HashSet<String> = Gaming::ALL.iter().map(|g| g.label()).collect();
        let pages: HashSet<&str> = Gaming::ALL.iter().map(|g| g.page()).collect();
        assert_eq!((labels.len(), pages.len()), (3, 3));
        assert!(pages.iter().all(|p| p.starts_with("ms-settings:")));
        for g in Gaming::ALL {
            g.enabled(); // reads the registry, whatever the answer
        }
        assert!(!Gaming::GpuScheduling.switchable());
        assert!(Gaming::GpuScheduling.set(true).is_err());
    }

    #[test]
    fn registry_dword_round_trip() {
        use windows_sys::Win32::System::Registry::RegDeleteTreeW;
        const KEY: &str = r"Software\CmdBoard-tests";
        set_dword(KEY, "value", 7).unwrap();
        assert_eq!(dword(HKEY_CURRENT_USER, KEY, "value"), Some(7));
        assert_eq!(dword(HKEY_CURRENT_USER, KEY, "missing"), None);
        // SAFETY: a nul-terminated key name under HKCU.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, programs::wide(KEY).as_ptr()) };
        assert_eq!(dword(HKEY_CURRENT_USER, KEY, "value"), None);
    }

    #[test]
    fn cpu_and_memory_measure_something() {
        let short = Duration::from_millis(50);
        assert!(cpu(1, short) > 0.0);
        assert!(cpu(2, short) > 0.0);
        let Ok(Score::Bytes(rate)) = memory(1 << 20, short) else {
            panic!("memory benchmark failed");
        };
        assert!(rate > 0.0);
    }

    #[test]
    fn reads_this_pc() {
        let sys = system();
        assert!(
            !sys.cpu.is_empty() && sys.threads > 0 && sys.ram > 0,
            "{sys:?}"
        );
        for setting in Gaming::ALL {
            setting.enabled(); // must not panic, whatever the registry holds
        }
    }
}
