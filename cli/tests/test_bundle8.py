"""Tests for bundle8 with a stand-in compressor (zlib). The real Oodle path runs under Windows/Wine."""

import io
import struct
import sys
import tempfile
import unittest
import zlib
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bundle8  # noqa: E402
import reforge_cli  # noqa: E402


class FakeOodle:
    def compress(self, raw, level=4):
        return b"Z" + zlib.compress(raw)

    def decompress(self, block, size=bundle8.CHUNK):
        assert block[:1] == b"Z"
        out = zlib.decompress(block[1:])
        assert len(out) == size
        return out


def make_bundle(logical: bytes, compressed_first=False) -> bytes:
    chunks = (len(logical) + bundle8.CHUNK - 1) // bundle8.CHUNK
    full = logical + bytes(chunks * bundle8.CHUNK - len(logical))
    blocks = [full[i * bundle8.CHUNK:(i + 1) * bundle8.CHUNK] for i in range(chunks)]
    if compressed_first:
        blocks[0] = FakeOodle().compress(blocks[0])
    out = bytearray(bundle8.MAGIC + struct.pack("<I", 1) + bytes(256) + struct.pack("<QQI", 1, 2, 0))
    out += struct.pack("<I", chunks) + b"".join(struct.pack("<I", len(b)) for b in blocks)
    out += bytes((-len(out)) % 16) + struct.pack("<II", len(logical), 0)
    for block in blocks:
        out += struct.pack("<I", len(block))
        out += bytes((-len(out)) % 16)
        out += block
    return bytes(out)


class Bundle8Tests(unittest.TestCase):
    def test_pack_compresses_stored_chunks_only(self):
        logical = bytes(range(256)) * 3000  # 768000 bytes -> 2 chunks
        data = make_bundle(logical, compressed_first=True)
        first_block = bundle8.parse(data).blocks[0]
        self.assertEqual(bundle8.stored_chunks(data), 1)
        packed = bundle8.pack(data, FakeOodle())
        layout = bundle8.parse(packed)
        self.assertEqual(bundle8.stored_chunks(packed), 0)
        self.assertEqual(layout.blocks[0], first_block)
        self.assertEqual(layout.length, len(logical))
        restored = b"".join(FakeOodle().decompress(b) for b in layout.blocks)[:layout.length]
        self.assertEqual(restored, logical)

    def test_rejects_malformed(self):
        data = make_bundle(b"x" * 100)
        with self.assertRaises(bundle8.BundleError):
            bundle8.parse(data + b"trailing")
        with self.assertRaises(bundle8.BundleError):
            bundle8.parse(data[:-10])
        self.assertEqual(bundle8.stored_chunks(b"not a bundle"), 0)

    def test_build_refuses_stored_bundle_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            game = Path(tmp) / "game"
            (game / "bundle").mkdir(parents=True)
            (game / "binaries").mkdir()
            (game / "bundle" / "aaaaaaaaaaaaaaaa").write_bytes(b"STOCK")
            mod = Path(tmp) / "MyMod"
            out = io.StringIO()
            with redirect_stdout(out), redirect_stderr(out):
                reforge_cli.main(["init", str(mod)])
                reforge_cli.main(["add", str(mod), "aaaaaaaaaaaaaaaa", "--game", str(game)])
                (mod / "payload" / "aaaaaaaaaaaaaaaa").write_bytes(make_bundle(b"y" * 1000))
                code = reforge_cli.main(["build", str(mod)])
            self.assertEqual(code, 2)
            self.assertIn("reforge pack", out.getvalue())
            (mod / "payload" / "aaaaaaaaaaaaaaaa").write_bytes(bundle8.pack(make_bundle(b"y" * 1000), FakeOodle()))
            with redirect_stdout(out), redirect_stderr(out):
                code = reforge_cli.main(["build", str(mod)])
            self.assertEqual(code, 0, out.getvalue())


if __name__ == "__main__":
    unittest.main()
