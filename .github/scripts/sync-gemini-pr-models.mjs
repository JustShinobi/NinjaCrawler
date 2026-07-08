import { appendFileSync } from "node:fs";
import { execFileSync } from "node:child_process";

const MODELS_ENDPOINT = "https://generativelanguage.googleapis.com/v1beta/models";
const LABEL_PREFIX = "pr-model/";
const DEFAULT_INCLUDE_PATTERN = "^gemini-";
const DEFAULT_EXCLUDE_PATTERN = "(embedding|image|imagen|veo|tts|live|native-audio|aqa|deprecated|shutdown)";
const DEFAULT_MODEL = "gemini-3.5-flash";

function fail(message) {
  console.error(`Error: ${message}`);
  process.exit(1);
}

function env(name) {
  return process.env[name] ?? "";
}

function appendSummary(markdown) {
  const summaryPath = env("GITHUB_STEP_SUMMARY");
  if (!summaryPath) {
    return;
  }

  appendFileSync(summaryPath, `${markdown}\n`, "utf8");
}

function runGh(args, options = {}) {
  return execFileSync("gh", args, {
    encoding: "utf8",
    maxBuffer: 4 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
  }).trim();
}

function readVariable(name) {
  const repo = env("GITHUB_REPOSITORY").trim();
  if (!repo) {
    return "";
  }

  try {
    return runGh(["variable", "get", name, "--repo", repo]);
  } catch {
    return "";
  }
}

function parseLines(value) {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith("#"));
}

function sortModels(models) {
  return [...models].sort((left, right) => {
    const rank = (model) => {
      if (model === DEFAULT_MODEL) return 0;
      if (model.includes("pro")) return 1;
      if (model.includes("flash") && !model.includes("lite")) return 2;
      if (model.includes("flash-lite")) return 3;
      return 4;
    };

    return rank(left) - rank(right) || left.localeCompare(right);
  });
}

async function listGeminiModels(apiKey) {
  const models = [];
  let pageToken = "";

  do {
    const url = new URL(MODELS_ENDPOINT);
    url.searchParams.set("pageSize", "1000");
    if (pageToken) {
      url.searchParams.set("pageToken", pageToken);
    }

    const response = await fetch(url, {
      headers: {
        "x-goog-api-key": apiKey,
      },
    });

    if (!response.ok) {
      let details = "";
      try {
        const errorBody = await response.json();
        details = errorBody?.error?.message ? ` ${errorBody.error.message}` : "";
      } catch {
        details = "";
      }
      fail(`Gemini models.list failed with HTTP ${response.status}.${details}`);
    }

    const payload = await response.json();
    models.push(...(payload.models ?? []));
    pageToken = payload.nextPageToken ?? "";
  } while (pageToken);

  return models;
}

function filterModels(models) {
  const includePattern = new RegExp(
    env("PR_MODEL_SYNC_INCLUDE_PATTERN") || readVariable("PR_MODEL_SYNC_INCLUDE_PATTERN") || DEFAULT_INCLUDE_PATTERN,
    "i",
  );
  const excludePattern = new RegExp(
    env("PR_MODEL_SYNC_EXCLUDE_PATTERN") || readVariable("PR_MODEL_SYNC_EXCLUDE_PATTERN") || DEFAULT_EXCLUDE_PATTERN,
    "i",
  );
  const pinnedModels = parseLines(env("PR_MODEL_SYNC_PINNED_MODELS") || readVariable("PR_MODEL_SYNC_PINNED_MODELS"));
  const modelByName = new Map(
    models.map((model) => [String(model.name ?? "").replace(/^models\//, ""), model]),
  );

  const discovered = models
    .map((model) => String(model.name ?? "").replace(/^models\//, ""))
    .filter((name) => name.length > 0)
    .filter((name) => includePattern.test(name))
    .filter((name) => !excludePattern.test(name))
    .filter((name) => {
      const methods = Array.isArray(modelByName.get(name)?.supportedGenerationMethods)
        ? modelByName.get(name).supportedGenerationMethods
        : [];
      return methods.length === 0 || methods.includes("generateContent") || methods.includes("interact");
    });

  return sortModels([...new Set([...pinnedModels, ...discovered])]);
}

function setVariable(name, value) {
  const repo = env("GITHUB_REPOSITORY");
  if (!repo) {
    fail("GITHUB_REPOSITORY is required.");
  }

  runGh(["variable", "set", name, "--repo", repo, "--body", value]);
}

function listExistingModelLabels() {
  const repo = env("GITHUB_REPOSITORY");
  const raw = runGh([
    "label",
    "list",
    "--repo",
    repo,
    "--limit",
    "500",
    "--json",
    "name",
    "--jq",
    `.[] | select(.name | startswith("${LABEL_PREFIX}")) | .name`,
  ]);

  return new Set(parseLines(raw));
}

function syncLabels(models) {
  const repo = env("GITHUB_REPOSITORY");
  const desiredLabels = new Set(models.map((model) => `${LABEL_PREFIX}${model}`));
  const existingLabels = listExistingModelLabels();
  const createdOrUpdated = [];
  const removed = [];

  for (const label of desiredLabels) {
    const model = label.slice(LABEL_PREFIX.length);
    const args = [
      "label",
      "create",
      label,
      "--repo",
      repo,
      "--color",
      "5319E7",
      "--description",
      `Regenerate PR title and body with ${model}`,
    ];

    try {
      runGh(args);
    } catch {
      runGh([
        "label",
        "edit",
        label,
        "--repo",
        repo,
        "--color",
        "5319E7",
        "--description",
        `Regenerate PR title and body with ${model}`,
      ]);
    }
    createdOrUpdated.push(label);
  }

  for (const label of existingLabels) {
    if (!desiredLabels.has(label)) {
      runGh(["label", "delete", label, "--repo", repo, "--yes"]);
      removed.push(label);
    }
  }

  return { createdOrUpdated, removed };
}

async function main() {
  const apiKey = env("GEMINI_API_KEY").trim();
  if (!apiKey) {
    fail("GEMINI_API_KEY must be configured.");
  }

  const models = filterModels(await listGeminiModels(apiKey));
  if (models.length === 0) {
    fail("No Gemini PR models matched the sync filters.");
  }

  setVariable("PR_ALLOWED_MODELS", models.join("\n"));
  const labels = syncLabels(models);

  appendSummary([
    "## Gemini PR model sync",
    "",
    "### Allowed models",
    ...models.map((model) => `- \`${model}\``),
    "",
    `Updated labels: ${labels.createdOrUpdated.length}`,
    `Removed labels: ${labels.removed.length}`,
  ].join("\n"));
}

main().catch((error) => {
  console.error(`Error: ${error instanceof Error ? error.message : "Unexpected failure."}`);
  process.exit(1);
});
