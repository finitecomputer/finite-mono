import { promises as fs } from "node:fs";
import path from "node:path";

export type BaselineSkillCatalogEntry = {
  id: string;
  name: string;
  description: string;
  category: string;
  managedRelpath: string;
  version: string | null;
  setupLabels: string[];
};

export type BaselineSkillsCatalogModel = {
  schemaVersion: string;
  totalSkillCount: number;
  categoryCount: number;
  skills: BaselineSkillCatalogEntry[];
};

type SkillSourceFile = {
  relativePath: string;
  text: string;
};

type GitHubTreeResponse = {
  tree?: Array<{
    path?: string;
    type?: string;
  }>;
};

const DEFAULT_FINITE_SKILLS_REPO = "finitecomputer/finite-skills";
const DEFAULT_FINITE_SKILLS_REF = "main";
const SKILLS_CATALOG_CACHE_TTL_MS = 5 * 60 * 1000;

let skillsSourceCache:
  | {
      key: string;
      expiresAt: number;
      promise: Promise<SkillSourceFile[]>;
    }
  | null = null;

function repoRoot() {
  return process.env.FC_REPO_ROOT
    ? path.resolve(process.env.FC_REPO_ROOT)
    : path.resolve(process.cwd(), "../..");
}

function humanizeCategory(value: string) {
  return value
    .split(/[-_]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function splitFrontmatter(raw: string) {
  if (!raw.startsWith("---\n")) {
    return { frontmatter: "", body: raw };
  }

  const end = raw.indexOf("\n---\n", 4);
  if (end === -1) {
    return { frontmatter: "", body: raw };
  }

  return {
    frontmatter: raw.slice(4, end),
    body: raw.slice(end + 5),
  };
}

function parseSimpleFrontmatter(block: string) {
  const result: Record<string, string> = {};
  const lines = block.split("\n");

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index]!;
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#") || /^\s/.test(line)) {
      continue;
    }

    const match = trimmed.match(/^([A-Za-z0-9_-]+):\s*(.*)$/);
    if (!match) {
      continue;
    }

    const [, key, rawValue] = match;
    const blockScalar = rawValue.match(/^([>|])[-+]?$/);
    if (blockScalar) {
      const content: string[] = [];
      while (index + 1 < lines.length) {
        const next = lines[index + 1]!;
        if (next.trim() && !/^\s/u.test(next)) break;
        index += 1;
        content.push(next.trim());
      }
      const value = blockScalar[1] === ">"
        ? content.filter(Boolean).join(" ")
        : content.join("\n").trim();
      if (value) result[key] = value;
      continue;
    }

    const value = rawValue.replace(/^['"]|['"]$/g, "").trim();
    if (value) result[key] = value;
  }

  return result;
}

async function walkSkillDirs(root: string): Promise<string[]> {
  const skillDirs: string[] = [];

  async function walk(dir: string): Promise<void> {
    const entries = await fs.readdir(dir, { withFileTypes: true });
    if (entries.some((entry) => entry.isFile() && entry.name === "SKILL.md")) {
      skillDirs.push(dir);
      return;
    }
    for (const entry of entries) {
      if (!entry.isDirectory()) {
        continue;
      }
      await walk(path.join(dir, entry.name));
    }
  }

  await walk(root);
  return skillDirs.sort((left, right) => left.localeCompare(right));
}

async function loadLocalSkillSources(root: string): Promise<SkillSourceFile[]> {
  const skillsRoot = path.basename(root) === "skills" ? root : path.join(root, "skills");
  const skillDirs = await walkSkillDirs(skillsRoot);
  return Promise.all(
    skillDirs.map(async (skillDir) => ({
      relativePath: path.relative(skillsRoot, skillDir).split(path.sep).join("/"),
      text: await fs.readFile(path.join(skillDir, "SKILL.md"), "utf8"),
    }))
  );
}

function finiteSkillsRepo() {
  return process.env.FC_FINITE_SKILLS_GITHUB_REPO || DEFAULT_FINITE_SKILLS_REPO;
}

function finiteSkillsRef() {
  return process.env.FC_FINITE_SKILLS_REF || DEFAULT_FINITE_SKILLS_REF;
}

function rawGitHubFileUrl(repo: string, ref: string, filePath: string) {
  const [owner, name] = repo.split("/", 2);
  if (!owner || !name) {
    throw new Error(`Invalid GitHub repo '${repo}'. Expected owner/name.`);
  }
  const encodedPath = filePath
    .split("/")
    .filter(Boolean)
    .map((part) => encodeURIComponent(part))
    .join("/");
  return `https://raw.githubusercontent.com/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/${encodeURIComponent(ref)}/${encodedPath}`;
}

function githubTreeUrl(repo: string, ref: string) {
  const [owner, name] = repo.split("/", 2);
  if (!owner || !name) {
    throw new Error(`Invalid GitHub repo '${repo}'. Expected owner/name.`);
  }
  return `https://api.github.com/repos/${encodeURIComponent(owner)}/${encodeURIComponent(name)}/git/trees/${encodeURIComponent(ref)}?recursive=1`;
}

async function fetchRawText(url: string): Promise<string> {
  const response = await fetch(url, {
    headers: {
      Accept: "text/plain",
      "User-Agent": "finitecomputer-dashboard",
    },
  });
  if (!response.ok) {
    throw new Error(`GitHub raw returned ${response.status} for ${url}`);
  }
  return response.text();
}

async function fetchGitHubSkillPaths(repo: string, ref: string): Promise<string[]> {
  const url = githubTreeUrl(repo, ref);
  const response = await fetch(url, {
    headers: {
      Accept: "application/vnd.github+json",
      "User-Agent": "finitecomputer-dashboard",
    },
  });
  if (!response.ok) {
    throw new Error(`GitHub tree returned ${response.status} for ${url}`);
  }

  const body = (await response.json()) as GitHubTreeResponse;
  return (body.tree ?? [])
    .filter((entry) => entry.type === "blob")
    .map((entry) => entry.path ?? "")
    .filter((entryPath) => entryPath.startsWith("skills/") && entryPath.endsWith("/SKILL.md"))
    .map((entryPath) => entryPath.slice("skills/".length, -"/SKILL.md".length))
    .sort((left, right) => left.localeCompare(right));
}

async function loadGitHubSkillSources(repo: string, ref: string): Promise<SkillSourceFile[]> {
  const skillPaths = await fetchGitHubSkillPaths(repo, ref);
  return Promise.all(
    skillPaths.map(async (relativePath) => {
      const skillPath = `skills/${relativePath}/SKILL.md`;
      return {
        relativePath,
        text: await fetchRawText(rawGitHubFileUrl(repo, ref, skillPath)),
      };
    })
  );
}

async function existingSiblingFiniteSkillsSourceDir() {
  const sibling = path.resolve(repoRoot(), "../finite-skills");
  try {
    await fs.access(path.join(sibling, "skills"));
    return sibling;
  } catch {
    return null;
  }
}

async function loadSkillSources(): Promise<SkillSourceFile[]> {
  const localSourceDir = process.env.FC_FINITE_SKILLS_SOURCE_DIR;
  if (localSourceDir) {
    return loadLocalSkillSources(path.resolve(localSourceDir));
  }

  const siblingSourceDir = await existingSiblingFiniteSkillsSourceDir();
  if (siblingSourceDir) {
    return loadLocalSkillSources(siblingSourceDir);
  }

  const repo = finiteSkillsRepo();
  const ref = finiteSkillsRef();
  const key = `${repo}@${ref}`;
  const now = Date.now();
  if (skillsSourceCache && skillsSourceCache.key === key && skillsSourceCache.expiresAt > now) {
    return skillsSourceCache.promise;
  }

  const promise = loadGitHubSkillSources(repo, ref);
  skillsSourceCache = {
    key,
    expiresAt: now + SKILLS_CATALOG_CACHE_TTL_MS,
    promise,
  };
  try {
    return await promise;
  } catch (error) {
    if (skillsSourceCache?.promise === promise) {
      skillsSourceCache = null;
    }
    throw error;
  }
}

export async function loadBaselineSkillsCatalog(): Promise<BaselineSkillsCatalogModel> {
  const skillSources = await loadSkillSources();

  const entries = await Promise.all(
    skillSources.map(async ({ relativePath, text }) => {
      const managedRelpath = `.hermes/skills/${relativePath}`;
      const { frontmatter, body } = splitFrontmatter(text);
      const meta = parseSimpleFrontmatter(frontmatter);
      const category = humanizeCategory(relativePath.split("/")[0] ?? "other");

      return {
        id: meta.name ?? path.basename(relativePath),
        name: meta.name ?? path.basename(relativePath),
        description:
          meta.description ??
          body
            .split("\n")
            .map((line) => line.trim())
            .find((line) => line.length > 0 && !line.startsWith("#")) ??
          "No description yet.",
        category,
        managedRelpath,
        version: meta.version ?? null,
        setupLabels: [],
      } satisfies BaselineSkillCatalogEntry;
    })
  );

  const skills = entries.sort((left, right) => left.name.localeCompare(right.name));

  return {
    schemaVersion: "finite-skills-tree-v1",
    totalSkillCount: skills.length,
    categoryCount: new Set(skills.map((entry) => entry.category)).size,
    skills,
  };
}
