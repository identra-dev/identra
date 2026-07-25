// Prints what detection finds and the launch PATH it would build, so the Finder/GUI stripped-PATH
// case can be driven by hand: run it under `env -i HOME=$HOME PATH=/usr/bin:/bin` and confirm the
// installed agents still resolve to absolute paths and the launch PATH leads with each one's dir.
fn main() {
    let found = identra_core::detect();
    for a in &found {
        if a.available {
            println!("{:<10} cmd={}", a.id, a.cmd);
            println!(
                "           launch PATH[0..1]={}",
                identra_core::agents::launch_path(&a.cmd)
                    .split(':')
                    .next()
                    .unwrap_or("")
            );
            println!(
                "           signed_in={} bus_wired={} reads_brief={}",
                a.logged_in, a.bus_wired, a.reads_connect_instructions
            );
        } else {
            println!("{:<10} (missing)", a.id);
        }
    }
    // Which agent the command center will stand up here, and why. "Why is it running that one"
    // is a real question with a data answer, and the three flags above are the whole of it.
    match identra_core::agents::best_orchestrator(&found) {
        Some(seat) => println!("\ncommand center seat -> {} ({})", seat.name, seat.id),
        None => println!("\ncommand center seat -> none: nothing here is installed and bus wired"),
    }
}
