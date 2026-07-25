// Prints what detection finds and the launch PATH it would build, so the Finder/GUI stripped-PATH
// case can be driven by hand: run it under `env -i HOME=$HOME PATH=/usr/bin:/bin` and confirm the
// installed agents still resolve to absolute paths and the launch PATH leads with each one's dir.
fn main() {
    for a in identra_core::detect() {
        if a.available {
            println!("{:<10} cmd={}", a.id, a.cmd);
            println!(
                "           launch PATH[0..1]={}",
                identra_core::agents::launch_path(&a.cmd)
                    .split(':')
                    .next()
                    .unwrap_or("")
            );
        } else {
            println!("{:<10} (missing)", a.id);
        }
    }
}
