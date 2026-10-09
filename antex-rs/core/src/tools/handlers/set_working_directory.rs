use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use antex_tools::JsonSchema;
use antex_tools::ResponsesApiTool;
use antex_tools::ToolName;
use antex_tools::ToolSpec;
use serde::Deserialize;
use std::collections::BTreeMap;

pub struct SetWorkingDirectoryHandler;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetWorkingDirectoryArgs {
    path: String,
}

impl ToolExecutor<ToolInvocation> for SetWorkingDirectoryHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("set_working_directory")
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec::Function(ResponsesApiTool {
            name: "set_working_directory".to_string(),
            description: "Change this session's working directory starting with the next model step and persist it for session restoration. Automatically call this after creating or selecting a worktree that you adopt for the current task, including worktrees created with shell commands. Use an absolute path to an existing directory. Do not use this for a temporary directory change within one command. Available only with a single local execution environment. Call this alone: end the current functions.exec cell after it and wait for its result before issuing dependent commands in a new model step. Calls already issued in this step retain the previous directory.".to_string(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(
                BTreeMap::from([(
                    "path".to_string(),
                    JsonSchema::string(Some("Absolute path to the session's new working directory.".to_string())),
                )]),
                Some(vec!["path".to_string()]),
                Some(false.into()),
            ),
            output_schema: None,
        })
    }

    fn handle<'a>(&'a self, invocation: ToolInvocation) -> antex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(async move {
            let ToolPayload::Function { arguments } = &invocation.payload else {
                return Err(FunctionCallError::RespondToModel(
                    "set_working_directory handler received unsupported payload".to_string(),
                ));
            };
            let args: SetWorkingDirectoryArgs = parse_arguments(arguments)?;
            let result = invocation
                .session
                .set_working_directory(invocation.step_context.as_ref(), &args.path)
                .await?;
            Ok(boxed_tool_output(FunctionToolOutput::from_text(
                result,
                Some(true),
            )))
        })
    }
}

impl CoreToolRuntime for SetWorkingDirectoryHandler {
    fn is_builtin_control_tool(&self) -> bool {
        true
    }
}
