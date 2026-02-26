import threading

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
