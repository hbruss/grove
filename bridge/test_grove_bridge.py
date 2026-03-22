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


if __name__ == "__main__":
    unittest.main()
