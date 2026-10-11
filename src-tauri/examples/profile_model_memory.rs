//! Local-only resource audit. Does not record audio or send AI requests.
#[cfg(windows)]
fn main() {
    use std::time::Duration;
    use windows_sys::Win32::{System::{ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},Threading::{GetCurrentProcess,GetProcessTimes}},Foundation::FILETIME};
    fn snapshot(stage: &str) {
        unsafe {
            let process = GetCurrentProcess();
            let mut memory: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
            let size = std::mem::size_of_val(&memory) as u32;
            memory.cb=size;
            assert_ne!(GetProcessMemoryInfo(process, (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),size),0);
            println!("{{\"stage\":\"{stage}\",\"privateMiB\":{:.1},\"workingMiB\":{:.1}}}", memory.PrivateUsage as f64 / 1048576.0,memory.WorkingSetSize as f64 / 1048576.0);
        }
    }
    fn cpu() -> u64 {
        unsafe {
            let mut creation: FILETIME=std::mem::zeroed(); let mut exit=creation; let mut kernel=creation; let mut user=creation;
            assert_ne!(GetProcessTimes(GetCurrentProcess(),&mut creation,&mut exit,&mut kernel,&mut user),0);
            (((kernel.dwHighDateTime as u64)<<32) | kernel.dwLowDateTime as u64) + (((user.dwHighDateTime as u64)<<32) | user.dwLowDateTime as u64)
        }
    }
    let dir=std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("com.typr.app");
    snapshot("empty");
    typr_lib::transcribe_nemotron::prewarm(&dir.join(typr_lib::transcribe_nemotron::model_dir_name("v3_5"))).unwrap();
    snapshot("nemotron_loaded");
    let start=cpu(); std::thread::sleep(Duration::from_secs(2));
    println!("{{\"stage\":\"nemotron_idle\",\"cpuPercentOneCore\":{:.2}}}",(cpu()-start) as f64 / 200000.0);
    typr_lib::transcribe_parakeet::prewarm(&dir.join(typr_lib::transcribe_parakeet::model_dir_name("v3"))).unwrap();
    snapshot("both_loaded_old_behavior");
    typr_lib::model_cache::reap("parakeet");
    snapshot("parakeet_selected_nemotron_released");
    typr_lib::model_cache::reap("cloud");
    snapshot("all_released");
}
#[cfg(not(windows))] fn main() { eprintln!("This resource audit uses Windows process counters."); }
