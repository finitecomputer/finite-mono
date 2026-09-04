import hashlib
from pathlib import Path
import shutil
import sqlite3
import stat
import subprocess
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "scripts" / "snapshot-sqlite"
BACKUPS_NIX = ROOT / "infra" / "nixos" / "modules" / "backups.nix"


class SnapshotSqliteTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scratch = tempfile.TemporaryDirectory()
        self.root = Path(self.scratch.name)

    def tearDown(self) -> None:
        for path in self.root.rglob("*"):
            if not path.is_symlink():
                path.chmod(path.stat().st_mode | stat.S_IWUSR)
        self.scratch.cleanup()

    def make_wal_snapshot(self, *, include_shm: bool = False) -> tuple[Path, Path]:
        live_dir = self.root / "live"
        snapshot = self.root / "snapshot"
        database = snapshot / "finite-sites" / "registry.db"
        live_dir.mkdir()
        database.parent.mkdir(parents=True)
        live_database = live_dir / "registry.db"

        connection = sqlite3.connect(live_database)
        connection.execute("CREATE TABLE evidence (value TEXT NOT NULL)")
        connection.commit()
        connection.close()

        connection = sqlite3.connect(live_database)
        self.assertEqual(
            connection.execute("PRAGMA journal_mode=WAL").fetchone()[0], "wal"
        )
        connection.execute("PRAGMA wal_autocheckpoint=0")
        connection.execute("INSERT INTO evidence VALUES ('uncheckpointed')")
        connection.commit()
        self.assertTrue(Path(f"{live_database}-wal").is_file())
        self.assertTrue(Path(f"{live_database}-shm").is_file())
        shutil.copy2(live_database, database)
        shutil.copy2(Path(f"{live_database}-wal"), Path(f"{database}-wal"))
        if include_shm:
            shutil.copy2(Path(f"{live_database}-shm"), Path(f"{database}-shm"))
        connection.close()

        (snapshot / "format").write_text(
            "finite.hosted-web-chat-recovery-snapshot.v3\n", encoding="utf-8"
        )
        entries = []
        for path in sorted(snapshot.rglob("*")):
            if path.is_file():
                relative = path.relative_to(snapshot)
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                entries.append(f"{digest}  {relative}\n")
        (snapshot / "manifest.sha256").write_text("".join(entries), encoding="utf-8")
        for path in sorted(snapshot.rglob("*"), reverse=True):
            if not path.is_symlink():
                path.chmod(path.stat().st_mode & ~0o222)
        snapshot.chmod(snapshot.stat().st_mode & ~0o222)
        return snapshot, database

    def tree_evidence(self, snapshot: Path) -> list[tuple[str, str, int]]:
        evidence = []
        for path in sorted(snapshot.rglob("*")):
            if path.is_file():
                evidence.append(
                    (
                        str(path.relative_to(snapshot)),
                        hashlib.sha256(path.read_bytes()).hexdigest(),
                        stat.S_IMODE(path.stat().st_mode),
                    )
                )
        return evidence

    def run_helper(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(HELPER), *args],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_helper_preserves_sealed_wal_snapshot_and_plain_sqlite_fails(self) -> None:
        snapshot, database = self.make_wal_snapshot()
        before = self.tree_evidence(snapshot)

        plain = subprocess.run(
            ["sqlite3", str(database), "SELECT value FROM evidence;"],
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertNotEqual(plain.returncode, 0, plain.stdout + plain.stderr)
        self.assertFalse(Path(f"{database}-shm").exists())

        query = self.run_helper("query", str(database), "SELECT value FROM evidence;")
        self.assertEqual(query.returncode, 0, query.stderr)
        self.assertEqual(query.stdout, "uncheckpointed\n")
        integrity = self.run_helper("integrity-check", str(database))
        self.assertEqual(integrity.returncode, 0, integrity.stderr)
        self.assertEqual(integrity.stdout, "ok\n")

        manifest = subprocess.run(
            ["sha256sum", "--check", "manifest.sha256"],
            cwd=snapshot,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(manifest.returncode, 0, manifest.stdout + manifest.stderr)
        self.assertEqual(self.tree_evidence(snapshot), before)

    def test_helper_copies_manifested_shm_and_rejects_other_paths(self) -> None:
        _, database = self.make_wal_snapshot(include_shm=True)
        query = self.run_helper(
            "query", str(database), "SELECT count(*) FROM evidence;"
        )
        self.assertEqual(query.returncode, 0, query.stderr)
        self.assertEqual(query.stdout, "1\n")

        outside = self.root / "outside.db"
        shutil.copy2(database, outside)
        refused = self.run_helper("integrity-check", str(outside))
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("not inside a manifested snapshot", refused.stderr)

        attach = self.run_helper(
            "query",
            str(database),
            f"ATTACH '{outside}' AS outside; SELECT 1;",
        )
        self.assertNotEqual(attach.returncode, 0)
        self.assertIn("cannot run ATTACH in safe mode", attach.stderr)

    def test_sealed_snapshot_remains_readable_and_rotation_unseals_it(self) -> None:
        snapshot, _ = self.make_wal_snapshot()
        archive = self.root / "snapshot.tar"
        with tarfile.open(archive, "w") as tar:
            tar.add(snapshot, arcname="snapshot", recursive=True)
        self.assertGreater(archive.stat().st_size, 0)

        subprocess.run(
            ["chmod", "-R", "u+w", "--", str(snapshot)],
            check=True,
            capture_output=True,
            text=True,
        )
        shutil.rmtree(snapshot)
        self.assertFalse(snapshot.exists())

        module = BACKUPS_NIX.read_text(encoding="utf-8")
        verify = module.index("sha256sum --check manifest.sha256")
        seal = module.index('chmod -R a-w -- "$staging"')
        activate = module.index('mv "$staging" "$final"')
        self.assertLess(verify, seal)
        self.assertLess(seal, activate)
        self.assertIn('remove_tree "$expired"', module)


if __name__ == "__main__":
    unittest.main()
