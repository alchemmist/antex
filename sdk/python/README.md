# Antex Python SDK

Build Python applications that start Antex threads, run turns, stream progress,
and control workspace access.

## Install

Install the SDK:

```bash
pip install antex-sdk
```

## Quickstart

The SDK reuses your existing Antex authentication when one is already
available:

```python
from antex_sdk import Antex

with Antex() as codex:
    thread = codex.thread_start()
    result = thread.run("Explain this repository in three bullets.")
    print(result.final_response)
```

`thread.run(...)` returns a `TurnResult` containing the final response,
collected items, and token usage.

## Authentication

Existing Antex authentication is reused automatically. To start ChatGPT
browser login explicitly:

```python
from antex_sdk import Antex

with Antex() as codex:
    login = codex.login_chatgpt()
    print(login.auth_url)
    print(login.wait().success)
```

For device-code login:

```python
with Antex() as codex:
    login = codex.login_chatgpt_device_code()
    print(login.verification_url, login.user_code)
    login.wait()
```

For API-key login:

```python
with Antex() as codex:
    codex.login_api_key("sk-...")
```

## Built-In Help

Use Python's standard `help(antex_sdk)`, `help(Antex)`, or
`python -m pydoc antex_sdk` documentation tools.

## Documentation

- [Getting started](https://github.com/openai/codex/blob/main/sdk/python/docs/getting-started.md)
- [API reference](https://github.com/openai/codex/blob/main/sdk/python/docs/api-reference.md)
- [FAQ](https://github.com/openai/codex/blob/main/sdk/python/docs/faq.md)
- [Examples](https://github.com/openai/codex/blob/main/sdk/python/examples/README.md)

The package is licensed under the
[repository Apache License 2.0](https://github.com/openai/codex/blob/main/LICENSE).
