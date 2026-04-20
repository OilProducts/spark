import threading

import pytest

from attractor.engine.context import Context


class TestContextLocking:
    def test_get_can_proceed_while_another_reader_holds_lock(self):
        context = Context(values={"key": "value"})
        read_completed = threading.Event()

        with context.lock.read_lock():
            thread = threading.Thread(
                target=lambda: (context.get("key"), read_completed.set()),
                daemon=True,
            )
            thread.start()
            assert read_completed.wait(timeout=1.0)

        thread.join(timeout=1.0)
        assert not thread.is_alive()

    def test_set_waits_until_reader_releases_lock(self):
        context = Context(values={"key": "value"})
        write_completed = threading.Event()

        with context.lock.read_lock():
            thread = threading.Thread(
                target=lambda: (context.set("key", "updated"), write_completed.set()),
                daemon=True,
            )
            thread.start()
            assert not write_completed.wait(timeout=0.1)

        assert write_completed.wait(timeout=1.0)
        thread.join(timeout=1.0)
        assert context.get("key") == "updated"

    def test_get_waits_until_writer_releases_lock(self):
        context = Context(values={"key": "value"})
        read_completed = threading.Event()

        with context.lock.write_lock():
            thread = threading.Thread(
                target=lambda: (context.get("key"), read_completed.set()),
                daemon=True,
            )
            thread.start()
            assert not read_completed.wait(timeout=0.1)

        assert read_completed.wait(timeout=1.0)
        thread.join(timeout=1.0)


class TestContextHelpers:
    def test_set_get_and_get_string(self):
        context = Context()
        context.set("name", "spark")
        context.set("attempts", 3)
        context.set("empty", None)

        assert context.get("name") == "spark"
        assert context.get("missing") is None
        assert context.get("missing", "fallback") == "fallback"
        assert context.get_string("name") == "spark"
        assert context.get_string("attempts") == "3"
        assert context.get("empty") is None
        assert context.get_string("empty", "none") == "none"
        assert context.get_string("missing", "fallback") == "fallback"
        assert "empty" not in context.snapshot()

    def test_append_log_and_clone_isolation(self):
        context = Context(values={"status": "ready"})
        context.append_log("first")

        cloned = context.clone()
        cloned.set("status", "running")
        cloned.append_log("second")

        assert context.get("status") == "ready"
        assert context.logs == ["first"]
        assert cloned.logs == ["first", "second"]

    def test_snapshot_returns_copy(self):
        context = Context(values={"a": 1})

        snapshot = context.snapshot()
        snapshot["a"] = 2
        snapshot["b"] = 3

        assert context.get("a") == 1
        assert context.get("b", None) is None

    def test_apply_updates_merges_values(self):
        context = Context(values={"a": 1})

        context.apply_updates({"a": 2, "b": "three"})

        assert context.get("a") == 2
        assert context.get("b") == "three"

    def test_apply_updates_none_removes_existing_keys(self):
        context = Context(values={"context.item.id": "ITEM-1", "context.item.title": "Ship it"})

        context.apply_updates({"context.item.id": None, "context.item.title": "Retitled", "context.item.summary": None})

        assert context.get("context.item.id") is None
        assert context.get("context.item.title") == "Retitled"
        assert context.get("context.item.summary") is None
        assert "context.item.id" not in context.snapshot()


class TestContextNamespaces:
    def test_allows_documented_namespaces_and_unscoped_keys(self):
        context = Context()

        context.set("plain_key", "ok")
        context.apply_updates(
            {
                "context.plan": "ready",
                "graph.goal": "ship",
                "internal.retry_count.task": 1,
                "parallel.results": [],
                "stack.supervisor": "manager",
                "human.gate.selected": "A",
                "work.item.id": "42",
                "_attractor.runtime.fidelity": "compact",
            }
        )

        assert context.get("plain_key") == "ok"
        assert context.get("context.plan") == "ready"
        assert context.get("graph.goal") == "ship"
        assert context.get("internal.retry_count.task") == 1
        assert context.get("parallel.results") == []
        assert context.get("stack.supervisor") == "manager"
        assert context.get("human.gate.selected") == "A"
        assert context.get("work.item.id") == "42"
        assert context.get("_attractor.runtime.fidelity") == "compact"

    def test_rejects_unknown_dotted_namespaces(self):
        context = Context(values={"context.keep": "yes"})

        with pytest.raises(ValueError, match="Unsupported context key namespace"):
            context.set("unknown.value", "nope")

        with pytest.raises(ValueError, match="Unsupported context key namespace"):
            context.apply_updates({"context.ok": "yes", "tool.output": "blocked"})

        assert context.get("context.keep") == "yes"
        assert context.get("context.ok") is None
        assert context.get("tool.output") is None
