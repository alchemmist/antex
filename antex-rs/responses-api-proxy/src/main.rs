use antex_responses_api_proxy::Args as ResponsesApiProxyArgs;
use clap::Parser;

#[ctor::ctor]
fn pre_main() {
    antex_process_hardening::pre_main_hardening();
}

pub fn main() -> anyhow::Result<()> {
    let args = ResponsesApiProxyArgs::parse();
    antex_responses_api_proxy::run_main(args)
}
