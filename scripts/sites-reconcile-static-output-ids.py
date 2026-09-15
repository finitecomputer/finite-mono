#!/usr/bin/env python3
"""One-time, offline output-ID reconciliation for the September 2026 copy.

Produces a new legacy registry copy, NOT a bootable static-only recovery set.
Never opens the input in SQLite. See docs/research/sites-output-exceptions.md.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import stat
import tempfile


class Refused(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Refused(message)


def sha256(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def no_links(path):
    path = path.absolute()
    require(all(not p.is_symlink() for p in (path, *path.parents)),
            'symlink paths are not supported')
    return path.resolve()


def quote(identifier):
    return '"' + identifier.replace('"', '""') + '"'


def state_digest(db, renames=()):
    """Compare every logical row and schema object, allowing only selected IDs.

    Sorting by every column is deterministic; project_outputs.id is its unique
    first column, so changing output_id cannot change row order.
    """
    digest = hashlib.sha256()
    schema = db.execute('SELECT * FROM sqlite_master ORDER BY type, name').fetchall()
    digest.update(repr(schema).encode())
    for pragma in ('user_version', 'application_id'):
        digest.update(repr(db.execute('PRAGMA ' + pragma).fetchall()).encode())
    tables = db.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
    for (table,) in tables.fetchall():
        columns = [r[1] for r in db.execute('PRAGMA table_info(' + quote(table) + ')')]
        order = ', '.join(quote(c) for c in columns)
        digest.update(repr((table, columns)).encode())
        for row in db.execute('SELECT * FROM ' + quote(table) + ' ORDER BY ' + order):
            if table == 'project_outputs' and row[columns.index('id')] in renames:
                row = list(row)
                row[columns.index('output_id')] = 'site'
                row = tuple(row)
            digest.update(repr(row).encode())
            digest.update(b'\n')
    return digest.digest()


def repository_present(root, project):
    require(isinstance(project, str) and re.fullmatch(r'[A-Za-z0-9_-]{1,128}', project),
            'unsupported project identifier')
    path = root / (project + '.git')
    return path.is_dir() and not path.is_symlink()


def inspect(db, repositories):
    outputs = db.execute('SELECT project_id, output_id, kind FROM project_outputs').fetchall()
    groups = {}
    for project, output, kind in outputs:
        groups.setdefault(project, []).append((output, kind))
    static_projects = {p for p, rows in groups.items() if any(k == 'site' for _, k in rows)}
    return {
        'noncanonical_static_outputs': sum(k == 'site' and o != 'site' for _, o, k in outputs),
        'mixed_projects': sum(any(k != 'site' for _, k in groups[p]) for p in static_projects),
        'unavailable_static_repositories': sum(not repository_present(repositories, p)
                                                for p in static_projects),
        'cutover_ready': False,
    }


def selection_ids(db, path, repositories):
    require(path.stat().st_size <= 8192, 'selection exceeds the one-time limit')
    selection = json.loads(path.read_text())
    require(isinstance(selection, list) and 1 <= len(selection) <= 3,
            'select one to three explicit outputs')
    ids = set()
    for item in selection:
        require(isinstance(item, dict) and set(item) == {
            'project_id', 'project_output_id', 'site_id', 'from_output_id',
        }, 'selection must contain exact project, output row, site and previous output IDs')
        require(all(isinstance(v, str) and re.fullmatch(r'[A-Za-z0-9_-]{1,128}', v)
                    for v in item.values()), 'invalid selection identifier')
        require(item['from_output_id'] in ('mockup', 'static', 'web'),
                'output ID is outside this one-time reconciliation')
        require(item['project_output_id'] not in ids, 'duplicate selection')
        rows = db.execute('''
            SELECT po.id, po.output_id, po.kind, po.site_id, po.document_entry,
                   po.start_command, s.kind, s.status, s.active_version_id
            FROM project_outputs po LEFT JOIN sites s ON s.id=po.site_id
            WHERE po.project_id=?
        ''', (item['project_id'],)).fetchall()
        require(len(rows) == 1, 'selected project is mixed or has multiple outputs')
        row = rows[0]
        require(row[:4] == (item['project_output_id'], item['from_output_id'],
                            'site', item['site_id']), 'selection does not match source state')
        require(row[4:8] == (None, None, 'static', 'published') and row[8] is not None,
                'selected output must be a published static site without runtime fields')
        require(repository_present(repositories, item['project_id']),
                'selected source repository is unavailable')
        ids.add(item['project_output_id'])
    return ids


def run(args):
    source = no_links(args.registry)
    repositories = no_links(args.repositories)
    require(source.is_file() and stat.S_ISREG(source.stat().st_mode),
            'registry must be a regular file')
    require(source.stat().st_size <= 128 * 1024 * 1024, 'registry exceeds the one-time size limit')
    require(repositories.is_dir(), 'repository directory is missing')
    require(re.fullmatch(r'[0-9a-f]{64}', args.expect_sha256), 'expected SHA-256 is invalid')
    # This receipt's registry is checkpointed. WAL recovery belongs to the
    # snapshot protocol, not this narrowly scoped conversion.
    for suffix in ('-wal', '-shm', '-journal'):
        sidecar = Path(str(source) + suffix)
        require(not sidecar.is_symlink() and (not sidecar.exists() or
                (sidecar.is_file() and sidecar.stat().st_size == 0)),
                'nonempty or unsafe SQLite sidecar requires snapshot handling')
    require(sha256(source) == args.expect_sha256, 'source hash mismatch')
    destination = None
    if args.mode == 'convert':
        require(args.selection is not None and args.output_dir is not None,
                'convert requires selection and a new output directory')
        destination = no_links(args.output_dir)
        require(not destination.exists(), 'output directory already exists')
        require(destination.parent.is_dir(), 'output parent must already exist')
        require(not destination.is_relative_to(source.parent),
                'output must be outside the input registry directory')
        require(not destination.is_relative_to(repositories),
                'output must be outside the source repositories directory')
    with tempfile.TemporaryDirectory(prefix='sites-output-reconciliation-') as temp:
        copy = Path(temp) / 'registry.db'
        shutil.copyfile(source, copy)
        require(sha256(copy) == args.expect_sha256, 'copied registry hash mismatch')
        db = sqlite3.connect(copy.as_uri() + '?mode=rw', uri=True)
        try:
            db.execute('PRAGMA trusted_schema=OFF')
            db.execute('PRAGMA foreign_keys=ON')
            require(db.execute('PRAGMA integrity_check').fetchall() == [('ok',)],
                    'registry integrity check failed')
            require(not db.execute('PRAGMA foreign_key_check').fetchall(),
                    'registry has foreign key violations')
            report = inspect(db, repositories)
            if destination is not None:
                sql = db.execute("SELECT sql FROM sqlite_master WHERE name='project_outputs'").fetchone()[0]
                require("'app'" in sql and "'document'" in sql,
                        'conversion requires the legacy schema before startup removes mixed outputs')
                require(not db.execute("SELECT 1 FROM sqlite_master WHERE type='trigger'").fetchall(),
                        'unexpected triggers require separate review')
                columns = db.execute('PRAGMA table_info(project_outputs)').fetchall()
                require(columns[0][1] == 'id' and columns[0][5] == 1,
                        'unexpected project output primary key')
                ids = selection_ids(db, args.selection, repositories)
                expected = state_digest(db, ids)
                db.execute('BEGIN IMMEDIATE')
                for output in sorted(ids):
                    result = db.execute("UPDATE project_outputs SET output_id='site' WHERE id=?", (output,))
                    require(result.rowcount == 1, 'selected output changed during conversion')
                require(state_digest(db) == expected, 'unexpected logical state change')
                require(not db.execute('PRAGMA foreign_key_check').fetchall(),
                        'converted registry has foreign key violations')
                db.commit()
                # Make the standalone output independent of scratch sidecars.
                db.execute('PRAGMA wal_checkpoint(TRUNCATE)')
                db.execute('PRAGMA journal_mode=DELETE')
                require(db.execute('PRAGMA integrity_check').fetchall() == [('ok',)],
                        'converted registry integrity check failed')
                require(state_digest(db) == expected, 'committed state differs')
                report = inspect(db, repositories)
                report.update(changed_outputs=len(ids), other_registry_state_preserved=True)
        finally:
            db.close()
        require(sha256(source) == args.expect_sha256, 'source changed during inspection')
        if destination is not None:
            destination.mkdir(mode=0o700)
            try:
                shutil.copyfile(copy, destination / 'registry.db')
                (destination / 'registry.db').chmod(0o600)
            except Exception:
                shutil.rmtree(destination)
                raise
        return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('inspect', 'convert'))
    parser.add_argument('--registry', type=Path, required=True)
    parser.add_argument('--expect-sha256', required=True)
    parser.add_argument('--repositories', type=Path, required=True)
    parser.add_argument('--selection', type=Path)
    parser.add_argument('--output-dir', type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    try:
        print(json.dumps(run(args), sort_keys=True))
    except Refused as error:
        print(json.dumps({'refused': str(error)}))
        return 1
    except (OSError, sqlite3.Error, ValueError, KeyError, TypeError, IndexError):
        # Paths, SQL and source values may contain customer data.
        print(json.dumps({'refused': 'unsupported input or offline inspection failed'}))
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
