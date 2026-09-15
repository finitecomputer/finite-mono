"""Synthetic fixtures, exercised only through the offline command contract."""
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
COMMAND = ROOT / 'scripts/sites-reconcile-static-output-ids.py'


class StaticOutputReconciliationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.source = self.root / 'source'
        self.source.mkdir()
        self.registry = self.source / 'registry.db'
        self.repositories = self.source / 'git/projects'
        self.repositories.mkdir(parents=True)
        self.destination = self.root / 'candidate'
        self.selection = self.root / 'selection.json'

    def fixture(self, output_id='mockup', mixed=False, missing=False):
        with sqlite3.connect(self.registry) as db:
            self.addCleanup(db.close)
            db.executescript('''
                CREATE TABLE projects (id TEXT PRIMARY KEY, owner_principal_id TEXT);
                CREATE TABLE sites (
                    id TEXT PRIMARY KEY, kind TEXT, status TEXT, active_version_id TEXT,
                    publisher_email_principal_id TEXT, originating_publisher_principal_id TEXT
                );
                CREATE TABLE project_outputs (
                    id TEXT PRIMARY KEY, project_id TEXT REFERENCES projects(id),
                    output_id TEXT, kind TEXT CHECK (kind IN ('site','app','document')),
                    site_id TEXT REFERENCES sites(id), document_entry TEXT, start_command TEXT,
                    UNIQUE(project_id, output_id), UNIQUE(site_id)
                );
                CREATE TABLE git_ref_events (
                    id INTEGER PRIMARY KEY, project_output_id TEXT REFERENCES project_outputs(id)
                );
                CREATE TABLE shares (site_id TEXT REFERENCES sites(id), email TEXT);
                INSERT INTO projects VALUES ('synthetic-project', 'synthetic-owner');
                INSERT INTO sites VALUES ('synthetic-site', 'static', 'published',
                    'synthetic-version', 'synthetic-email-principal', 'synthetic-origin');
                INSERT INTO shares VALUES ('synthetic-site', 'viewer@example.test');
                INSERT INTO git_ref_events VALUES (1, 'synthetic-output');
            ''')
            db.execute('INSERT INTO project_outputs VALUES (?, ?, ?, ?, ?, NULL, NULL)',
                       ('synthetic-output', 'synthetic-project', output_id, 'site', 'synthetic-site'))
            if mixed:
                db.executescript('''
                    INSERT INTO sites VALUES ('synthetic-app', 'app', 'published',
                        'synthetic-app-version', 'synthetic-email-principal', 'synthetic-origin');
                    INSERT INTO project_outputs VALUES ('synthetic-app-output',
                        'synthetic-project', 'app', 'app', 'synthetic-app', NULL, 'node app.js');
                ''')
            if missing:
                db.execute("UPDATE sites SET status='claimed_unpublished', active_version_id=NULL")
        if not missing:
            (self.repositories / 'synthetic-project.git').mkdir()
        self.selection.write_text(json.dumps([{
            'project_id': 'synthetic-project', 'project_output_id': 'synthetic-output',
            'site_id': 'synthetic-site', 'from_output_id': output_id,
        }]))

    def run_command(self, mode, registry=None, *extra):
        registry = registry or self.registry
        result = subprocess.run([
            sys.executable, str(COMMAND), mode, '--registry', str(registry),
            '--expect-sha256', hashlib.sha256(registry.read_bytes()).hexdigest(),
            '--repositories', str(self.repositories), *extra,
        ], capture_output=True, text=True, cwd=ROOT)
        return result

    def convert(self):
        return self.run_command('convert', None, '--selection', str(self.selection),
                                '--output-dir', str(self.destination))

    def test_convert_preserves_identity_history_and_source_bytes(self):
        self.fixture()
        before = self.registry.read_bytes()
        result = self.convert()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        receipt = json.loads(result.stdout)
        self.assertEqual(receipt['changed_outputs'], 1)
        self.assertTrue(receipt['other_registry_state_preserved'])
        after = self.run_command('inspect', self.destination / 'registry.db')
        self.assertEqual(after.returncode, 0, after.stderr)
        self.assertEqual(json.loads(after.stdout)['noncanonical_static_outputs'], 0)
        self.assertEqual(self.registry.read_bytes(), before)
        artifact_copy = self.root / 'artifact-inspection.db'
        shutil.copyfile(self.destination / 'registry.db', artifact_copy)
        db = sqlite3.connect(artifact_copy.as_uri() + '?mode=ro', uri=True)
        try:
            self.assertEqual(db.execute('SELECT * FROM project_outputs').fetchall(), [
                ('synthetic-output', 'synthetic-project', 'site', 'site',
                 'synthetic-site', None, None),
            ])
            self.assertEqual(db.execute('SELECT * FROM projects').fetchall(), [
                ('synthetic-project', 'synthetic-owner'),
            ])
            self.assertEqual(db.execute('SELECT * FROM sites').fetchall(), [
                ('synthetic-site', 'static', 'published', 'synthetic-version',
                 'synthetic-email-principal', 'synthetic-origin'),
            ])
            self.assertEqual(db.execute('SELECT * FROM shares').fetchall(), [
                ('synthetic-site', 'viewer@example.test'),
            ])
            self.assertEqual(db.execute('SELECT * FROM git_ref_events').fetchall(), [
                (1, 'synthetic-output'),
            ])
        finally:
            db.close()

    def test_mixed_static_app_project_is_reported_and_never_converted(self):
        self.fixture(output_id='static', mixed=True)
        before = self.registry.read_bytes()
        inspected = self.run_command('inspect')
        self.assertEqual(inspected.returncode, 0, inspected.stdout)
        self.assertEqual(json.loads(inspected.stdout)['mixed_projects'], 1)
        refused = self.convert()
        self.assertEqual(refused.returncode, 1)
        self.assertIn('mixed or has multiple outputs', refused.stdout)
        self.assertFalse(self.destination.exists())
        self.assertEqual(self.registry.read_bytes(), before)
        self.assertNotIn('synthetic-project', refused.stdout + refused.stderr)

    def test_output_inside_separate_repository_tree_is_refused_without_changes(self):
        self.fixture()
        separate = self.root / 'repositories'
        shutil.move(self.repositories, separate)
        self.repositories = separate
        repo = self.repositories / 'synthetic-project.git'
        (repo / 'HEAD').write_bytes(b'ref: refs/heads/main\n')
        (repo / 'objects').mkdir()
        (repo / 'objects/synthetic-object').write_bytes(b'synthetic repository bytes\x00')
        before = {str(p.relative_to(separate)): p.read_bytes() if p.is_file() else None
                  for p in separate.rglob('*')}
        registry_before = self.registry.read_bytes()
        for destination in (separate / 'candidate', repo / 'objects/candidate'):
            with self.subTest(destination=destination.relative_to(separate)):
                self.destination = destination
                refused = self.convert()
                self.assertFalse(destination.exists())
                self.assertEqual(
                    {str(p.relative_to(separate)): p.read_bytes() if p.is_file() else None
                     for p in separate.rglob('*')}, before)
                self.assertEqual(self.registry.read_bytes(), registry_before)
                self.assertEqual(refused.returncode, 1)
                self.assertIn('outside the source repositories directory', refused.stdout)

    def test_missing_unpublished_repository_is_not_fabricated(self):
        self.fixture(output_id='web', missing=True)
        before = self.registry.read_bytes()
        inspected = self.run_command('inspect')
        self.assertEqual(inspected.returncode, 0, inspected.stdout)
        self.assertEqual(json.loads(inspected.stdout)['unavailable_static_repositories'], 1)
        refused = self.convert()
        self.assertEqual(refused.returncode, 1)
        self.assertFalse(self.destination.exists())
        self.assertEqual(list(self.repositories.iterdir()), [])
        self.assertEqual(self.registry.read_bytes(), before)

    def test_replay_cannot_overwrite_or_reselect_converted_state(self):
        self.fixture(output_id='web')
        self.assertEqual(self.convert().returncode, 0)
        candidate = self.destination / 'registry.db'
        before = candidate.read_bytes()
        replay = self.convert()
        self.assertEqual(replay.returncode, 1)
        self.assertIn('already exists', replay.stdout)
        replay_output = self.root / 'second-candidate'
        reselection = self.run_command('convert', candidate, '--selection', str(self.selection),
                                      '--output-dir', str(replay_output))
        self.assertEqual(reselection.returncode, 1)
        self.assertIn('does not match source state', reselection.stdout)
        self.assertFalse(replay_output.exists())
        self.assertEqual(candidate.read_bytes(), before)

    def test_wrong_hash_and_nonempty_wal_are_refused_without_outputs(self):
        self.fixture()
        wrong_hash = self.run_command('convert', None, '--expect-sha256', '0' * 64,
                                     '--selection', str(self.selection),
                                     '--output-dir', str(self.destination))
        self.assertEqual(wrong_hash.returncode, 1)
        self.assertIn('source hash mismatch', wrong_hash.stdout)
        with sqlite3.connect(self.registry) as writer:
            self.addCleanup(writer.close)
            writer.execute('PRAGMA journal_mode=WAL')
            writer.execute('PRAGMA wal_autocheckpoint=0')
            writer.execute("INSERT INTO shares VALUES ('synthetic-site', 'another@example.test')")
            writer.commit()
            refused = self.convert()
            self.assertEqual(refused.returncode, 1)
            self.assertIn('sidecar requires snapshot handling', refused.stdout)
        self.assertFalse(self.destination.exists())

    def test_stale_selection_and_batch_failure_leave_source_unchanged(self):
        self.fixture()
        before = self.registry.read_bytes()
        selection = json.loads(self.selection.read_text())
        selection.append({**selection[0], 'project_id': 'absent-project',
                          'project_output_id': 'absent-output'})
        self.selection.write_text(json.dumps(selection))
        refused = self.convert()
        self.assertEqual(refused.returncode, 1)
        self.assertFalse(self.destination.exists())
        self.assertEqual(self.registry.read_bytes(), before)

    def test_source_symlink_is_refused_without_opening_it_in_sqlite(self):
        self.fixture()
        link = self.root / 'linked.db'
        link.symlink_to(self.registry)
        refused = self.run_command('inspect', link)
        self.assertEqual(refused.returncode, 1)
        self.assertIn('symlink paths', refused.stdout)
        self.assertFalse(Path(str(self.registry) + '-shm').exists())


if __name__ == '__main__':
    unittest.main()
