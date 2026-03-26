import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest

import bridge.grove_bridge as grove_bridge
from bridge.grove_bridge import (
    BridgeController,
    BridgeSession,
    InMemorySessionStore,
    Iterm2SessionStore,
    SessionLocationHint,
)


class BridgeControllerTests(unittest.IsolatedAsyncioTestCase):
    async def test_set_role_clears_same_role_in_same_window(self):
        store = InMemorySessionStore(
            [
                BridgeSession(
                    session_id="editor-1",
                    title="Editor One",
                    role="editor",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                ),
                BridgeSession(
                    session_id="editor-2",
                    title="Editor Two",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-2"),
                ),
            ]
        )
        controller = BridgeController(store)

        response = await controller.handle_envelope(
            {
                "request_id": "req-1",
                "command": {
                    "set_role": {"session_id": "editor-2", "role": "editor"},
                },
            }
        )

        self.assertEqual(response, {"request_id": "req-1", "response": "pong"})
        self.assertEqual(store.get_session("editor-1").role, None)
        self.assertEqual(store.get_session("editor-2").role, "editor")

    async def test_clear_role_removes_role_from_target_session(self):
        store = InMemorySessionStore(
            [
                BridgeSession(
                    session_id="editor-1",
                    title="Editor One",
                    role="editor",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                )
            ]
        )
        controller = BridgeController(store)

        response = await controller.handle_envelope(
            {
                "request_id": "req-clear",
                "command": {"clear_role": {"session_id": "editor-1"}},
            }
        )

        self.assertEqual(response, {"request_id": "req-clear", "response": "pong"})
        self.assertEqual(store.get_session("editor-1").role, None)

    async def test_list_sessions_returns_location_hints(self):
        store = InMemorySessionStore(
            [
                BridgeSession(
                    session_id="grove",
                    title="Grove",
                    role="grove",
                    instance_id="instance-1",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                ),
                BridgeSession(
                    session_id="ai-1",
                    title="Claude",
                    role="ai",
                    job_name="claude-code",
                    command_line="claude",
                    cwd="/repo",
                    location_hint=SessionLocationHint(
                        window_id="window-1",
                        tab_id="tab-2",
                        window_title="Workspace",
                        tab_title="AI",
                    ),
                )
            ]
        )
        controller = BridgeController(store)

        response = await controller.handle_envelope(
            {"request_id": "req-list", "command": {"list_sessions": {"instance_id": "instance-1"}}}
        )

        self.assertEqual(
            response,
            {
                "request_id": "req-list",
                "response": {
                    "session_list": [
                        {
                            "session_id": "ai-1",
                            "title": "Claude",
                            "role": "ai",
                            "job_name": "claude-code",
                            "command_line": "claude",
                            "cwd": "/repo",
                            "location_hint": {
                                "window_id": "window-1",
                                "tab_id": "tab-2",
                                "window_title": "Workspace",
                                "tab_title": "AI",
                            },
                        }
                    ]
                },
            },
        )

    async def test_list_sessions_hides_live_grove_sessions_but_keeps_stale_grove_tagged_codex(self):
        store = InMemorySessionStore(
            [
                BridgeSession(
                    session_id="grove",
                    title="Grove",
                    role="grove",
                    instance_id="instance-1",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                ),
                BridgeSession(
                    session_id="stale-grove",
                    title="Primary (codex)",
                    role="grove",
                    job_name="codex",
                    command_line="codex",
                    cwd="/repo",
                    instance_id="old-instance",
                    location_hint=SessionLocationHint(
                        window_id="window-1",
                        tab_id="tab-1",
                        window_title="Workspace",
                        tab_title="Primary",
                    ),
                ),
                BridgeSession(
                    session_id="other-grove",
                    title="Primary (zsh)",
                    role="grove",
                    job_name="zsh",
                    command_line="-zsh",
                    cwd="/repo",
                    instance_id="instance-2",
                    location_hint=SessionLocationHint(
                        window_id="window-1",
                        tab_id="tab-2",
                        window_title="Workspace",
                        tab_title="Primary",
                    ),
                ),
                BridgeSession(
                    session_id="ai-1",
                    title="Claude",
                    role="ai",
                    job_name="claude-code",
                    command_line="claude",
                    cwd="/repo",
                    location_hint=SessionLocationHint(
                        window_id="window-1",
                        tab_id="tab-2",
                        window_title="Workspace",
                        tab_title="AI",
                    ),
                ),
            ]
        )
        controller = BridgeController(store)

        response = await controller.handle_envelope(
            {"request_id": "req-stale", "command": {"list_sessions": {"instance_id": "instance-1"}}}
        )

        self.assertEqual(
            response,
            {
                "request_id": "req-stale",
                "response": {
                    "session_list": [
                        {
                            "session_id": "stale-grove",
                            "title": "Primary (codex)",
                            "role": None,
                            "job_name": "codex",
                            "command_line": "codex",
                            "cwd": "/repo",
                            "location_hint": {
                                "window_id": "window-1",
                                "tab_id": "tab-1",
                                "window_title": "Workspace",
                                "tab_title": "Primary",
                            },
                        },
                        {
                            "session_id": "ai-1",
                            "title": "Claude",
                            "role": "ai",
                            "job_name": "claude-code",
                            "command_line": "claude",
                            "cwd": "/repo",
                            "location_hint": {
                                "window_id": "window-1",
                                "tab_id": "tab-2",
                                "window_title": "Workspace",
                                "tab_title": "AI",
                            },
                        },
                    ]
                },
            },
        )

    async def test_resolve_targets_prefers_same_tab_then_same_window(self):
        store = InMemorySessionStore(
            [
                BridgeSession(
                    session_id="grove",
                    title="Grove",
                    role="grove",
                    instance_id="instance-1",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                ),
                BridgeSession(
                    session_id="ai-same-tab",
                    title="Claude",
                    role="ai",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                ),
                BridgeSession(
                    session_id="editor-same-window",
                    title="Helix",
                    role="editor",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-2"),
                ),
                BridgeSession(
                    session_id="ai-same-window",
                    title="Codex",
                    role="ai",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-3"),
                ),
            ]
        )
        controller = BridgeController(store)

        response = await controller.handle_envelope(
            {
                "request_id": "req-2",
                "command": {"resolve_targets": {"instance_id": "instance-1"}},
            }
        )

        self.assertEqual(
            response,
            {
                "request_id": "req-2",
                "response": {
                    "targets_resolved": {
                        "ai_target_session_id": "ai-same-tab",
                        "editor_target_session_id": "editor-same-window",
                        "source": "same_window",
                    }
                },
            },
        )

    async def test_resolve_targets_returns_same_tab_when_all_targets_are_in_tab(self):
        store = InMemorySessionStore(
            [
                BridgeSession(
                    session_id="grove",
                    title="Grove",
                    role="grove",
                    instance_id="instance-1",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                ),
                BridgeSession(
                    session_id="ai-same-tab",
                    title="Claude",
                    role="ai",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                ),
                BridgeSession(
                    session_id="editor-same-tab",
                    title="Helix",
                    role="editor",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                ),
            ]
        )
        controller = BridgeController(store)

        response = await controller.handle_envelope(
            {
                "request_id": "req-3",
                "command": {"resolve_targets": {"instance_id": "instance-1"}},
            }
        )

        self.assertEqual(
            response,
            {
                "request_id": "req-3",
                "response": {
                    "targets_resolved": {
                        "ai_target_session_id": "ai-same-tab",
                        "editor_target_session_id": "editor-same-tab",
                        "source": "same_tab",
                    }
                },
            },
        )

    async def test_send_text_returns_manual_selection_required_for_unresolved_role(self):
        store = InMemorySessionStore(
            [
                BridgeSession(
                    session_id="grove",
                    title="Grove",
                    role="grove",
                    instance_id="instance-1",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                )
            ]
        )
        controller = BridgeController(store)

        response = await controller.handle_envelope(
            {
                "request_id": "req-4",
                "command": {
                    "send_text": {
                        "instance_id": "instance-1",
                        "target": {"role": "ai"},
                        "text": "src/lib.rs",
                        "append_newline": True,
                    }
                },
            }
        )

        self.assertEqual(
            response,
            {
                "request_id": "req-4",
                "response": {"manual_selection_required": {"role": "ai"}},
            },
        )
        self.assertEqual(store.sent_text, [])

    async def test_send_text_returns_target_session_unavailable_for_missing_direct_session(self):
        store = InMemorySessionStore(
            [
                BridgeSession(
                    session_id="grove",
                    title="Grove",
                    role="grove",
                    instance_id="instance-1",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                )
            ]
        )
        controller = BridgeController(store)

        response = await controller.handle_envelope(
            {
                "request_id": "req-4b",
                "command": {
                    "send_text": {
                        "instance_id": "instance-1",
                        "target": {"session_id": "missing-session"},
                        "text": "src/lib.rs",
                        "append_newline": False,
                    }
                },
            }
        )

        self.assertEqual(
            response,
            {
                "request_id": "req-4b",
                "response": {
                    "target_session_unavailable": {
                        "session_id": "missing-session",
                    }
                },
            },
        )
        self.assertEqual(store.sent_text, [])

    async def test_iterm2_session_store_send_text_suppresses_broadcast(self):
        class FakeSession:
            def __init__(self) -> None:
                self.calls: list[tuple[str, bool]] = []

            async def async_send_text(
                self, text: str, suppress_broadcast: bool = False
            ) -> None:
                self.calls.append((text, suppress_broadcast))

        class FakeApp:
            def __init__(self, session: FakeSession) -> None:
                self._session = session

            def get_session_by_id(self, session_id: str) -> FakeSession | None:
                if session_id == "target-1":
                    return self._session
                return None

        fake_session = FakeSession()
        original_iterm2 = grove_bridge.iterm2
        
        async def fake_async_get_app(connection):
            return FakeApp(fake_session)

        grove_bridge.iterm2 = SimpleNamespace(
            async_get_app=fake_async_get_app
        )

        try:
            store = Iterm2SessionStore(connection=object())
            await store.send_text("target-1", "hello")
        finally:
            grove_bridge.iterm2 = original_iterm2

        self.assertEqual(fake_session.calls, [("hello", True)])

    async def test_iterm2_session_store_list_sessions_includes_minimized_sessions(self):
        class FakeSession:
            def __init__(
                self,
                session_id: str,
                title: str,
                *,
                role: str | None = None,
                job_name: str | None = None,
                command_line: str | None = None,
                cwd: str | None = None,
                instance_id: str | None = None,
            ) -> None:
                self.session_id = session_id
                self._values = {
                    "presentationName": title,
                    "name": title,
                    "user.groveRole": role,
                    "jobName": job_name,
                    "commandLine": command_line,
                    "path": cwd,
                    "user.groveInstance": instance_id,
                }

            async def async_get_variable(self, name: str) -> str | None:
                return self._values.get(name)

        class FakeTab:
            def __init__(
                self,
                tab_id: str,
                title: str,
                *,
                sessions: list[FakeSession],
                minimized_sessions: list[FakeSession],
            ) -> None:
                self.tab_id = tab_id
                self.sessions = sessions
                self.all_sessions = sessions + minimized_sessions
                self._title = title

            async def async_get_variable(self, name: str) -> str | None:
                if name == "title":
                    return self._title
                return None

        class FakeWindow:
            def __init__(self, window_id: str, tabs: list[FakeTab]) -> None:
                self.window_id = window_id
                self.tabs = tabs

        class FakeApp:
            def __init__(self, windows: list[FakeWindow]) -> None:
                self.windows = windows

        visible = FakeSession(
            "visible-1",
            "Claude",
            role="ai",
            job_name="claude",
            command_line="claude",
            cwd="/repo",
        )
        minimized = FakeSession(
            "minimized-1",
            "Codex",
            job_name="codex",
            command_line="codex",
            cwd="/repo",
        )
        fake_app = FakeApp(
            [
                FakeWindow(
                    "window-1",
                    [
                        FakeTab(
                            "tab-1",
                            "Workspace",
                            sessions=[visible],
                            minimized_sessions=[minimized],
                        )
                    ],
                )
            ]
        )
        original_iterm2 = grove_bridge.iterm2

        async def fake_async_get_app(connection):
            return fake_app

        grove_bridge.iterm2 = SimpleNamespace(async_get_app=fake_async_get_app)

        try:
            store = Iterm2SessionStore(connection=object())
            sessions = await store.list_sessions()
        finally:
            grove_bridge.iterm2 = original_iterm2

        self.assertEqual([session.session_id for session in sessions], ["visible-1", "minimized-1"])
        self.assertEqual(sessions[1].location_hint.tab_id, "tab-1")
        self.assertEqual(sessions[1].location_hint.tab_title, "Workspace")

    async def test_list_sessions_logs_inclusion_decisions(self):
        store = InMemorySessionStore(
            [
                BridgeSession(
                    session_id="grove",
                    title="Grove",
                    role="grove",
                    job_name="zsh",
                    command_line="-zsh",
                    instance_id="instance-1",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                ),
                BridgeSession(
                    session_id="stale-grove",
                    title="Primary (codex)",
                    role="grove",
                    job_name="codex",
                    command_line="codex",
                    cwd="/repo",
                    instance_id="old-instance",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-1"),
                ),
                BridgeSession(
                    session_id="other-grove",
                    title="Primary (zsh)",
                    role="grove",
                    job_name="zsh",
                    command_line="-zsh",
                    cwd="/repo",
                    instance_id="instance-2",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-2"),
                ),
                BridgeSession(
                    session_id="ai-1",
                    title="Claude",
                    role="ai",
                    job_name="claude-code",
                    command_line="claude",
                    cwd="/repo",
                    location_hint=SessionLocationHint(window_id="window-1", tab_id="tab-2"),
                ),
            ]
        )
        with tempfile.TemporaryDirectory() as tmp_dir:
            logger = grove_bridge.BridgeLogger(
                grove_bridge.BridgeDebugConfig(path=Path(tmp_dir) / "bridge.log")
            )
            controller = BridgeController(store, logger=logger)
            try:
                response = await controller.handle_envelope(
                    {
                        "request_id": "req-log",
                        "command": {"list_sessions": {"instance_id": "instance-1"}},
                    }
                )
            finally:
                logger.close()

            events = [
                json.loads(line)
                for line in (Path(tmp_dir) / "bridge.log").read_text(encoding="utf-8").splitlines()
            ]

        self.assertEqual(response["request_id"], "req-log")
        self.assertEqual(response["response"]["session_list"][0]["session_id"], "stale-grove")
        self.assertEqual(response["response"]["session_list"][1]["session_id"], "ai-1")

        normalized_events = [
            {key: value for key, value in event.items() if key != "ts_ms"} for event in events
        ]
        self.assertIn(
            {
                "event": "bridge.command_received",
                "request_id": "req-log",
                "command": "list_sessions",
            },
            normalized_events,
        )
        self.assertIn(
            {
                "event": "bridge.sender_resolved",
                "request_id": "req-log",
                "command": "list_sessions",
                "instance_id": "instance-1",
                "session_id": "grove",
            },
            normalized_events,
        )
        self.assertIn(
            {
                "event": "bridge.list_sessions_decision",
                "request_id": "req-log",
                "session_id": "stale-grove",
                "decision": "include",
                "reason": "stale_grove_target_markers",
            },
            normalized_events,
        )
        self.assertIn(
            {
                "event": "bridge.list_sessions_decision",
                "request_id": "req-log",
                "session_id": "other-grove",
                "decision": "exclude",
                "reason": "live_grove_session",
            },
            normalized_events,
        )
        self.assertIn(
            {
                "event": "bridge.list_sessions_decision",
                "request_id": "req-log",
                "session_id": "ai-1",
                "decision": "include",
                "reason": "eligible_session",
            },
            normalized_events,
        )


class BridgeLoggingTests(unittest.TestCase):
    def test_load_bridge_debug_config_returns_none_for_missing_file(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            config = grove_bridge.load_bridge_debug_config(
                Path(tmp_dir) / "missing-bridge-debug.json"
            )

        self.assertIsNone(config)

    def test_load_bridge_debug_config_returns_enabled_config_for_valid_file(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            config_path = Path(tmp_dir) / "bridge-debug.json"
            log_path = Path(tmp_dir) / "bridge.log"
            config_path.write_text(
                json.dumps({"path": str(log_path), "log_session_lists": True}),
                encoding="utf-8",
            )

            config = grove_bridge.load_bridge_debug_config(config_path)

        self.assertEqual(config.path, log_path)
        self.assertTrue(config.log_session_lists)


if __name__ == "__main__":
    unittest.main()
