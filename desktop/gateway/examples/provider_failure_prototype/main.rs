mod controller;

use std::io::{self, Write};

use controller::{AttemptController, RouteContext, RetryPolicy};

fn render(controller: &AttemptController) {
    print!("\x1b[2J\x1b[H");
    println!("\x1b[1mProvider Failure Contract Prototype\x1b[0m");
    println!("\x1b[2mNo network, OAuth, or persistence\x1b[0m\n");
    println!("\x1b[1mprovider\x1b[0m: {}", controller.context().provider);
    println!("\x1b[1mroute\x1b[0m: {}", controller.context().route);
    println!(
        "\x1b[1mcorrelation_id\x1b[0m: {}",
        controller.context().correlation_id
    );
    println!("\x1b[1mstate\x1b[0m: {:#?}", controller.state());
    println!("\n\x1b[1m[p]\x1b[0m begin POST  \x1b[1m[b]\x1b[0m bytes started");
    println!("\x1b[1m[r]\x1b[0m reset       \x1b[1m[q]\x1b[0m quit");
    print!("> ");
    io::stdout().flush().expect("flush prototype frame");
}

fn main() {
    let context = RouteContext {
        provider: "codex".into(),
        route: "responses_lite".into(),
        correlation_id: "prototype-0001".into(),
        policy: RetryPolicy::default(),
    };
    let mut controller = AttemptController::new(context);
    loop {
        render(&controller);
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }
        match input.trim() {
            "p" => {
                controller.begin_post();
            }
            "b" => {
                controller.mark_response_started();
            }
            "r" => controller.reset(),
            "q" => break,
            _ => {}
        }
    }
}
