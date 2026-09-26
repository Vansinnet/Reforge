"""Tests for the reforge CLI against a fake game folder. Run: python -m unittest discover cli/tests"""

import hashlib
import io
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import reforge_cli  # noqa: E402


def run(*argv):
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = reforge_cli.main(list(argv))
    return code, out.getvalue() + err.getvalue()


class CliTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        base = Path(self.tmp.name)
        self.game = base / "game"
        (self.game / "bundle" / "sub").mkdir(parents=True)
        (self.game / "binaries").mkdir()
        (self.game / "bundle" / "98bb14b1d247a0c8").write_bytes(b"STOCK")
        self.mod = base / "mods" / "MyMod"

    def tearDown(self):
        self.tmp.cleanup()

    def test_full_flow(self):
        code, out = run("init", str(self.mod), "--author", "Me")
        self.assertEqual(code, 0, out)
        for rel in ["MyMod.mod", "info.json", "bin/reforge.dll", "bin/REFORGE_NOTICES.txt", "scripts/mods/MyMod/reforge.lua",
                    "scripts/mods/MyMod/MyMod.lua", "scripts/mods/MyMod/reforge_manifest.lua", "reforge.json"]:
            self.assertTrue((self.mod / rel).is_file(), rel)
        self.assertNotIn("{{", (self.mod / "scripts/mods/MyMod/MyMod.lua").read_text())

        code, out = run("add", str(self.mod), "98bb14b1d247a0c8", "--copy", "--game", str(self.game))
        self.assertEqual(code, 0, out)
        payload = self.mod / "payload" / "98bb14b1d247a0c8"
        self.assertEqual(payload.read_bytes(), b"STOCK")
        payload.write_bytes(b"EDITED")

        (self.mod / "payload" / "mats").mkdir()
        (self.mod / "payload" / "mats" / "aa11").write_bytes(b"M")
        self.assertEqual(run("add-virtual-dir", str(self.mod), "bundle/data/mymod", "payload/mats")[0], 0)
        self.assertEqual(run("add-virtual", str(self.mod), "bundle/data/mymod2/x", "payload/mats/aa11")[0], 0)

        code, out = run("build", str(self.mod), "--game", str(self.game))
        self.assertEqual(code, 0, out)
        manifest = (self.mod / "scripts/mods/MyMod/reforge_manifest.lua").read_text()
        sha = hashlib.sha256(b"STOCK").hexdigest()
        self.assertIn(f'stock = "bundle/98bb14b1d247a0c8", file = "payload/98bb14b1d247a0c8", sha256 = "{sha}"', manifest)
        self.assertIn('stock = "bundle/data/mymod/aa11", file = "payload/mats/aa11", virtual = true', manifest)
        self.assertIn('stock = "bundle/data/mymod2/x"', manifest)

        luajit = subprocess.run(["luajit", "-e", f"local m = dofile([[{self.mod / 'scripts/mods/MyMod/reforge_manifest.lua'}]]) assert(m.schema == 1 and #m.redirects == 3)"])
        self.assertEqual(luajit.returncode, 0)

        self.assertEqual(run("verify", str(self.mod), "--game", str(self.game))[0], 0)
        (self.game / "bundle" / "98bb14b1d247a0c8").write_bytes(b"PATCHED")
        code, out = run("verify", str(self.mod), "--game", str(self.game))
        self.assertEqual(code, 1)
        self.assertIn("game updated?", out)
        self.assertEqual(run("build", str(self.mod), "--game", str(self.game))[0], 2)

    def test_rejects_bad_input(self):
        run("init", str(self.mod))
        code, out = run("add", str(self.mod), "../binaries/x", "--game", str(self.game))
        self.assertEqual(code, 2)
        self.assertIn("'..'", out)
        code, out = run("add", str(self.mod), "ABCDEF", "--game", str(self.game))
        self.assertIn("lowercase", out)
        code, out = run("add", str(self.mod), "missing", "--game", str(self.game))
        self.assertIn("does not exist", out)
        run("add-virtual", str(self.mod), "bundle/data/x", "payload/nothing")
        code, out = run("build", str(self.mod))
        self.assertEqual(code, 2)
        self.assertIn("payload payload/nothing is missing", out)
        code, out = run("init", str(self.mod))
        self.assertIn("not empty", out)
        code, out = run("init", str(self.mod.parent / "1bad"))
        self.assertIn("must start with a letter", out)

    def test_sync_refuses_downgrade(self):
        run("init", str(self.mod))
        lua = self.mod / "scripts/mods/MyMod/reforge.lua"
        lua.write_text(lua.read_text().replace("local LIB_VERSION = 1", "local LIB_VERSION = 99"))
        code, out = run("sync", str(self.mod))
        self.assertEqual(code, 2)
        self.assertIn("newer than this tool", out)


if __name__ == "__main__":
    unittest.main()
