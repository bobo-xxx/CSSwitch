mod controller;

use std::io::{self, Write};

use controller::{
    AttemptController, AttemptDirective, FailureObservation, ProtocolClass, RateKind, RepairKind,
    RouteContext,
};

fn http(
    status: u16,
    rate_kind: Option<RateKind>,
    retry_after_ms: Option<u64>,
) -> FailureObservation {
    FailureObservation::Http {
        status,
        rate_kind,
        retry_after_ms,
    }
}

fn render(controller: &AttemptController) {
    print!("\x1b[2J\x1b[H");
    println!("\x1b[1mProvider Failure Contract Prototype\x1b[0m");
    println!("\x1b[2mNo network, OAuth, proxy, or persistence\x1b[0m\n");
    println!("\x1b[1mprovider\x1b[0m: {}", controller.provider_label());
    println!("\x1b[1mroute\x1b[0m: {}", controller.route_label());
    println!(
        "\x1b[1mcorrelation_id\x1b[0m: {}",
        controller.correlation_id_label()
    );
    println!("\x1b[1mstate\x1b[0m: {:#?}", controller.state());
    println!(
        "\x1b[1mlast observation\x1b[0m: {:#?}",
        controller.last_observation()
    );
    println!(
        "\x1b[1mlast directive\x1b[0m: {:#?}",
        controller.last_directive()
    );
    println!(
        "\x1b[1mlast transition error\x1b[0m: {:#?}",
        controller.last_transition_error()
    );
    if let Some(AttemptDirective::Fail(failure)) = controller.last_directive() {
        println!(
            "\x1b[1menvelope\x1b[0m:\n{}",
            serde_json::to_string_pretty(&failure.anthropic_json())
                .expect("serialize prototype envelope")
        );
    }
    println!("\n\x1b[1m[p]\x1b[0m begin POST   \x1b[1m[b]\x1b[0m bytes started");
    println!("\x1b[1m[1]\x1b[0m capability   \x1b[1m[2]\x1b[0m 401   \x1b[1m[3]\x1b[0m 403");
    println!("\x1b[1m[4]\x1b[0m rate 429     \x1b[1m[5]\x1b[0m quota 429");
    println!("\x1b[1m[6]\x1b[0m network      \x1b[1m[7]\x1b[0m upstream 500");
    println!("\x1b[1m[8]\x1b[0m known repair \x1b[1m[9]\x1b[0m protocol");
    println!("\x1b[1m[408]\x1b[0m HTTP 408    \x1b[1m[409]\x1b[0m HTTP 409");
    println!("\x1b[1m[c]\x1b[0m cancel       \x1b[1m[r]\x1b[0m reset  \x1b[1m[q]\x1b[0m quit");
    print!("> ");
    io::stdout().flush().expect("flush prototype frame");
}

fn main() {
    let context = RouteContext::codex_responses_lite_prototype();
    let mut controller = AttemptController::new(
        context,
        Some(RepairKind::OmitUnsupportedAutomaticToolChoice),
    );
    loop {
        render(&controller);
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }
        match input.trim() {
            "p" => {
                let _result = controller.begin_post();
            }
            "b" => {
                let _result = controller.mark_response_started();
            }
            "1" => {
                let _result = controller.observe(FailureObservation::Capability);
            }
            "2" => {
                let _result = controller.observe(http(401, None, None));
            }
            "3" => {
                let _result = controller.observe(http(403, None, None));
            }
            "4" => {
                let _result = controller.observe(http(429, Some(RateKind::RateLimit), Some(1_500)));
            }
            "5" => {
                let _result = controller.observe(http(429, Some(RateKind::Quota), None));
            }
            "6" => {
                let _result = controller.observe(FailureObservation::Network);
            }
            "7" => {
                let _result = controller.observe(http(500, None, None));
            }
            "8" => {
                let _result = controller.observe(FailureObservation::Protocol(
                    ProtocolClass::UnsupportedAutomaticToolChoice,
                ));
            }
            "9" => {
                let _result = controller
                    .observe(FailureObservation::Protocol(ProtocolClass::InvalidResponse));
            }
            "408" => {
                let _result = controller.observe(http(408, None, None));
            }
            "409" => {
                let _result = controller.observe(http(409, None, None));
            }
            "c" => {
                let _result = controller.observe(FailureObservation::Cancelled);
            }
            "r" => controller.reset(),
            "q" => break,
            _ => {}
        }
    }
}
