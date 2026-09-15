import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, symlink, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import { root } from './operator.mjs';

for (const kind of ['hidden-file','symlink','tab-name','missing-index']) {
  test(`publisher rejects ${kind} before remote mutation`, async () => {
    const directory = await mkdtemp(join(tmpdir(),'finite-vercel-reject-'));
    try {
      const git = args => execFileSync('git',['-C',directory,...args],{stdio:'pipe'});
      git(['init','-b','main']);
      git(['config','user.name','Experiment']);
      git(['config','user.email','experiment@example.invalid']);
      await mkdir(join(directory,'site'));
      if (kind !== 'missing-index') await writeFile(join(directory,'site/index.html'),'Synthetic fixture');
      if (kind === 'hidden-file') await writeFile(join(directory,'site/.env'),'');
      if (kind === 'symlink') await symlink('index.html',join(directory,'site/link.html'));
      if (kind === 'tab-name') await writeFile(join(directory,'site/tab\tname.txt'),'');
      if (kind === 'missing-index') await writeFile(join(directory,'site/readme.txt'),'');
      git(['add','site']); git(['commit','-m','Invalid synthetic deploy tree']);
      const result = spawnSync(process.execPath,[join(root,'publish.mjs'),'reject-test',directory,'HEAD'],{encoding:'utf8'});
      assert.notEqual(result.status,0);
      assert.match(result.stderr,kind === 'missing-index' ? /needs index.html/ : /Unsupported deploy entry/);
    } finally { await rm(directory,{recursive:true,force:true}); }
  });
}
