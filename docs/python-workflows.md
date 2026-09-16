# Python workflows

Python workflows — это добавленная в этом форке система перезапускаемых программ оркестрации для больших повторяющихся задач, которые неудобно решать в одном контексте модели.

Сейчас вместе с Codex поставляются три готовых workflow:

- `Ruff cleanup` — исправляет большие очереди Ruff-нарушений;
- `GitHub bot PR maintenance` — обходит все репозитории выбранного владельца и по умолчанию исправляет и мержит безопасные bot PR.
- `PR babysitter` — дешёво опрашивает один PR и запускает чистого сильного агента только для новых CI failures или review feedback.

Их исходный код находится в `antex-rs/tui/src/workflow/builtin_ruff.py`, `antex-rs/tui/src/workflow/builtin_github_bot_pr_maintenance.py` и `antex-rs/tui/src/workflow/builtin_pr_babysitter.py`.

При запуске Codex материализует встроенный workflow во внутренний кэш:

```text
~/.antex/workflow-cache/ruff-cleanup-v1.py
~/.antex/workflow-cache/github-bot-pr-maintenance-v1.py
```

## Где хранить свои workflows

Личные workflows, доступные во всех проектах:

```text
~/.antex/workflows/*.py
```

Проектные workflows:

```text
<project>/.antex/workflows/*.py
```

Если несколько workflows имеют одинаковый `id`, используется следующий приоритет:

```text
project → personal → built-in
```

Таким образом, проект может переопределить личный или встроенный workflow без изменения исходников Codex.

## Структура workflow

Каждый Python-файл содержит manifest `WORKFLOW` и функцию `run(ctx)`:

```python
WORKFLOW = {
    "id": "my-workflow",
    "title": "My workflow",
    "description": "What it does",
    "fields": [],
    "guardrails": {},
}


def run(ctx):
    return {"result": "done"}
```

Manifest описывает интерфейс запуска и ограничения:

```python
WORKFLOW = {
    "id": "lint-cleanup",
    "title": "Lint cleanup",
    "description": "Fix lint errors with independent agents",
    "fields": [
        {
            "id": "scope",
            "label": "Scope",
            "type": "text",
            "default": "src",
            "required": True,
        },
        {
            "id": "parallelism",
            "label": "Parallel agents",
            "type": "integer",
            "min": 1,
            "max": 15,
            "default": 5,
        },
        {
            "id": "verify",
            "label": "Run final verification",
            "type": "boolean",
            "default": True,
        },
    ],
    "guardrails": {
        "max_agent_calls": 1000,
        "max_shell_calls": 1000,
        "max_parallel_agents": 15,
        "timeout_seconds": 43200,
    },
}
```

Поддерживаются четыре типа полей:

- `text`;
- `integer`;
- `boolean`;
- `select`.

На основании `fields` Codex сам строит TUI-форму при запуске `/workflow`. Python-код управляет содержанием формы, а Rust отвечает за её отображение и проверку значений.

## Workflow context API

Функция `run(ctx)` получает синхронный API оркестрации. Python описывает порядок действий и ветвление, а Codex выполняет асинхронную работу, запускает команды и управляет агентами.

```python
def run(ctx):
    scope = ctx.params["scope"]

    scan = ctx.shell(["ruff", "check", scope, "--output-format", "json"])

    result = ctx.agent(
        "Исправь эту конкретную ошибку",
        model="gpt-5.6-sol",
        reasoning_effort="high",
        developer_instructions="Never weaken quality gates.",
    )

    results = ctx.agent_batch(
        [
            {"prompt": "Fix issue one"},
            {"prompt": "Fix issue two"},
        ],
        parallelism=ctx.params["parallelism"],
        model="gpt-5.6-sol",
    )

    ctx.checkpoint({
        "completed": len(results),
    })

    ctx.progress(
        "Fixing violations",
        current=len(results),
        total=100,
    )

    ctx.log("Finished current batch")

    return {"fixed": len(results)}
```

Доступные операции:

- `ctx.params` — значения, введённые в TUI перед запуском;
- `ctx.state` — последний сохранённый checkpoint или пустой объект для нового запуска;
- `ctx.shell(argv, cwd=None, timeout_seconds=None, env=None)` — выполнение ограниченной shell-команды; команда передаётся списком аргументов без неявной интерпретации shell;
- `ctx.agent(prompt, model=None, reasoning_effort=None, developer_instructions=None, forbid_quality_graph_ignore=False, cwd=None, timeout_seconds=None)` — запуск одного эфемерного Codex-агента;
- `ctx.agent_batch(prompts, parallelism=None, model=None, reasoning_effort=None, developer_instructions=None, forbid_quality_graph_ignore=False, cwd=None, timeout_seconds=None)` — параллельный запуск независимых агентов;
- `ctx.checkpoint(json_value)` — сохранение состояния размером до 1 MiB;
- `ctx.progress(message, current=None, total=None)` — обновление статуса workflow в TUI;
- `ctx.log(message)` — диагностическая запись в журнал Codex без повреждения протокола workflow.

Каждый элемент `prompts` в `ctx.agent_batch` может быть строкой либо объектом с индивидуальными значениями `prompt`, `model`, `reasoning_effort`, `developer_instructions`, `forbid_quality_graph_ignore`, `cwd` и `timeout_seconds`.

Агенты запускаются через установленный локально `codex exec` с тем же `ANTEX_HOME`, авторизацией и подпиской. Каждый вызов эфемерный и начинает работу с небольшим независимым контекстом.

## Как работает Ruff cleanup

Встроенный workflow выполняет следующий цикл:

1. Запускает Ruff с JSON-выводом.
2. Группирует нарушения по файлам.
3. Выбирает не более одной задачи на файл в текущей волне, чтобы параллельные агенты не редактировали один файл одновременно.
4. Запускает агентов через `ctx.agent_batch()`.
5. Сохраняет checkpoint.
6. Снова запускает Ruff и получает актуальный список ошибок и номера строк.
7. Повторяет цикл, пока ошибки не закончатся или не сработает один из guardrails.

Перед запуском можно настроить:

- scope;
- команду Ruff;
- количество нарушений на одного агента;
- параллелизм;
- модель;
- допустимое количество неудачных запусков;
- максимальное количество проходов;
- финальную проверку.

Workflow запрещает агентам добавлять `noqa`, ignores, exclusions или ослаблять конфигурацию Ruff. Каждому агенту передаётся небольшой атомарный набор ошибок одного файла.

## Состояние и возобновление

Каждый запуск получает отдельную директорию:

```text
~/.antex/workflow-runs/<timestamp>-<8-symbol-uuid>/
```

Внутри находятся:

- `workflow.py` — снимок Python-кода на момент запуска;
- `params.json` — выбранные пользователем параметры;
- `state.json` — последний checkpoint;
- `run.json` — статус, ошибка и счётчики вызовов агентов и shell-команд.

Снимок `workflow.py` гарантирует, что возобновлённый запуск использует ту же версию программы, даже если исходный workflow уже был изменён.

В списке возобновления Codex показывает до 100 последних запусков со статусами `running`, `paused`, `failed` или `cancelled`.

## Команды управления

- `/workflow` — открыть список workflows и запустить выбранный;
- `/workflow pause` — остановить процесс на ближайшем host action и сохранить checkpoint;
- `/workflow stop` — отменить выполнение, сохранив состояние для диагностики или возобновления;
- `/workflow resume` — выбрать и продолжить приостановленный, упавший, отменённый или прерванный закрытием Codex запуск.

## Ограничения и безопасность

Rust проверяет manifest, ограничивает количество agent и shell calls, параллелизм, продолжительность выполнения, размер сообщений протокола и checkpoint. Рабочая директория действий не может выходить за пределы текущего workspace.

При этом Python-файл является доверенным локальным кодом. Сам интерпретатор Python не запускается в отдельной песочнице и технически может обращаться к стандартной библиотеке и API операционной системы. Поэтому следует запускать только workflows, исходному коду которых вы доверяете.
