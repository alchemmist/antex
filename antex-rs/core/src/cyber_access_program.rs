use antex_api::AccessPrograms;
use antex_login::AntexAuth;
use antex_protocol::turn_input::CyberAccessProgram;

pub(crate) fn for_auth(
    auth: Option<&AntexAuth>,
    program: Option<CyberAccessProgram>,
) -> Option<AccessPrograms> {
    program
        .filter(|_| auth.is_some_and(AntexAuth::is_chatgpt_auth))
        .map(AccessPrograms::from)
}
