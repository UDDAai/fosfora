fn main() {
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("native-video-harness runs on macOS");
        std::process::exit(2);
    }
    #[cfg(target_os = "macos")]
    {
        let code = match fosfora_native_video::bench::main() {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("{err}");
                1
            }
        };
        std::process::exit(code);
    }
}
