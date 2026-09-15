import { mkdir, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { state } from './operator.mjs';

const directory = join(state, 'source-fixture');
await mkdir(join(directory, 'site/assets'), { recursive: true });
const git = args => execFileSync('git', ['-C', directory, ...args], { encoding: 'utf8' }).trim();
git(['init', '-b', 'main']);
git(['config', 'user.name', 'Finite Sites Experiment']);
git(['config', 'user.email', 'experiment@example.invalid']);
const commits = [];
for (const version of [1, 2]) {
  await writeFile(join(directory, 'site/index.html'), `<!doctype html><html><head><meta charset="utf-8"><title>Finite / Private Site</title><link rel="stylesheet" href="/assets/style.css"></head><body><main><p class="eyebrow">FINITE / PRIVATE SITE</p><h1>Content version ${version}</h1><p>This page is served by Vercel. Finite checked your current permission before delivering it.</p><section><h2>What survives a rollback?</h2><p>Access rules live outside this deployment. Revoking a viewer blocks this page and its assets, including older versions.</p></section><p class="foot">Disposable experiment · Synthetic data only</p></main></body></html>`);
  await writeFile(join(directory, 'site/assets/style.css'), 'body{background:#f4f5ef;color:#20352b;font:20px/1.55 system-ui;margin:0}main{max-width:720px;margin:100px auto;padding:24px}.eyebrow,.foot{font-size:12px;letter-spacing:.12em}h1{font-size:64px;font-weight:500;line-height:1.1}h2{font-size:24px}section{border-top:1px solid #bdc8b9;margin-top:48px;padding-top:20px}');
  await writeFile(join(directory, 'site/assets/private.txt'), `PRIVATE-ASSET-V${version}\n`);
  await writeFile(join(directory, 'site/llms.txt'), 'Experiment source is the private Git repository; do not reconstruct source from HTML.\n');
  git(['add', 'site']);
  git(['commit', '--allow-empty', '-m', `Synthetic content version ${version}`]);
  commits.push(git(['rev-parse', 'HEAD']));
}
await writeFile(join(state, 'fixture.json'), JSON.stringify({ repository: directory, commits }, null, 2));
console.log(JSON.stringify({ repository: directory, commits }, null, 2));
