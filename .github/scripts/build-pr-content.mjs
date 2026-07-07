import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

const DEFAULT_MODEL = "gemini-3.5-flash";
const LABEL_PREFIX = "pr-model/";
const DEFAULT_DIFF_MAX_CHARS = 50000;
const INTERACTIONS_ENDPOINT = "https://generativelanguage.googleapis.com/v1beta/interactions";

function fail(message) {
  console.error(`Error: ${message}`);
  process.exit(1);
}

function env(name) {
  return process.env[name] ?? "";
}

function readVariable(name) {
  const repo = env("GITHUB_REPOSITORY").trim();
  if (!repo) {
    return "";
  }

  try {
    return execFileSync("gh", ["variable", "get", name, "--repo", repo], {
      encoding: "utf8",
      maxBuffer: 2 * 1024 * 1024,
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return "";
  }
}

function readConfig(name, { required = false } = {}) {
  const plain = env(name).trim();
  if (plain.length > 0) {
    return plain;
  }

  const encoded = env(`${name}_B64`).trim();
  if (encoded.length > 0) {
    try {
      return Buffer.from(encoded, "base64").toString("utf8").trim();
    } catch {
      fail(`${name}_B64 is not valid base64.`);
    }
  }

  const variablePlain = readVariable(name);
  if (variablePlain.length > 0) {
    return variablePlain;
  }

  const variableEncoded = readVariable(`${name}_B64`);
  if (variableEncoded.length > 0) {
    try {
      return Buffer.from(variableEncoded, "base64").toString("utf8").trim();
    } catch {
      fail(`${name}_B64 is not valid base64.`);
    }
  }

  if (required) {
    fail(`${name} or ${name}_B64 must be configured.`);
  }

  return "";
}

function appendOutput(name, value) {
  const outputPath = env("GITHUB_OUTPUT");
  if (!outputPath) {
    return;
  }

  const delimiter = `EOF_${name}_${Date.now()}_${Math.random().toString(16).slice(2)}`;
  appendFileSync(outputPath, `${name}<<${delimiter}\n${value}\n${delimiter}\n`, "utf8");
}

function appendSummary(markdown) {
  const summaryPath = env("GITHUB_STEP_SUMMARY");
  if (!summaryPath) {
    return;
  }

  appendFileSync(summaryPath, `${markdown}\n`, "utf8");
}

function normalizeBranch(value, name) {
  let branch = value.trim();
  if (!branch) {
    fail(`${name} is required.`);
  }

  branch = branch.replace(/^refs\/heads\//, "").replace(/^origin\//, "");
  if (!branch || branch.startsWith("-") || branch.includes("\0")) {
    fail(`${name} is not a valid branch name.`);
  }

  try {
    execFileSync("git", ["check-ref-format", "--branch", branch], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch {
    fail(`${name} is not a valid branch name.`);
  }

  return branch;
}

function validateModelName(model) {
  if (!/^[A-Za-z0-9._:/-]+$/.test(model)) {
    fail("Requested model contains unsupported characters.");
  }
}

function parseLines(value) {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith("#"));
}

function resolveModel() {
  const label = env("PR_MODEL_LABEL").trim();
  if (label) {
    if (!label.startsWith(LABEL_PREFIX)) {
      fail("Model label must start with pr-model/.");
    }

    const model = label.slice(LABEL_PREFIX.length).trim();
    if (!model) {
      fail("Model label does not include a model name.");
    }
    validateModelName(model);
    return model;
  }

  const selected = env("PR_MODEL_INPUT").trim() || readConfig("PR_GENERATOR_MODEL") || DEFAULT_MODEL;
  const model = selected === "custom" ? env("PR_CUSTOM_MODEL").trim() : selected;
  if (!model) {
    fail("custom_model is required when model is custom.");
  }

  validateModelName(model);
  return model;
}

function ensureAllowedModel(model) {
  const allowedConfig = readConfig("PR_ALLOWED_MODELS", { required: true });
  const allowedModels = parseLines(allowedConfig);
  if (!allowedModels.includes(model)) {
    fail("Requested model is not listed in PR_ALLOWED_MODELS.");
  }
}

function runGit(args, options = {}) {
  return execFileSync("git", args, {
    encoding: "utf8",
    maxBuffer: 20 * 1024 * 1024,
    ...options,
  }).trimEnd();
}

function fetchBranch(branch) {
  runGit([
    "fetch",
    "--no-tags",
    "--quiet",
    "origin",
    `+refs/heads/${branch}:refs/remotes/origin/${branch}`,
  ]);
}

function truncate(value, maxChars) {
  if (value.length <= maxChars) {
    return value;
  }

  return `${value.slice(0, maxChars)}\n\n[truncated ${value.length - maxChars} characters]`;
}

function collectContext(baseBranch, headBranch) {
  fetchBranch(baseBranch);
  fetchBranch(headBranch);

  const baseRef = `origin/${baseBranch}`;
  const headRef = `origin/${headBranch}`;
  const commitCount = Number(runGit(["rev-list", "--count", `${baseRef}..${headRef}`]));
  if (!Number.isFinite(commitCount) || commitCount < 1) {
    fail(`No commits found on ${headBranch} ahead of ${baseBranch}.`);
  }

  const diffMaxChars = Number(env("PR_DIFF_MAX_CHARS")) || DEFAULT_DIFF_MAX_CHARS;
  const commits = runGit([
    "log",
    "--no-merges",
    "--pretty=format:%H%n%s%n%b%n---END-COMMIT---",
    `${baseRef}..${headRef}`,
  ]);
  const diffStat = runGit(["diff", "--stat=160", `${baseRef}...${headRef}`]);
  const changedFiles = runGit(["diff", "--name-status", `${baseRef}...${headRef}`]);
  const diff = truncate(
    runGit(["diff", "--no-ext-diff", "--find-renames", "--unified=2", `${baseRef}...${headRef}`]),
    diffMaxChars,
  );

  return {
    baseBranch,
    headBranch,
    commitCount,
    commits,
    diffStat,
    changedFiles,
    diff,
  };
}

function buildPrompt(configuredPrompt, context) {
  const validationNotes = env("PR_VALIDATION_NOTES").trim();
  const { title: existingTitle, body: existingBody } = readExistingPullRequest();

  return [
    "Follow the configured pull request instructions exactly.",
    "Return only JSON matching the requested schema.",
    "",
    "Configured instructions:",
    configuredPrompt,
    "",
    "Pull request context:",
    `Base branch: ${context.baseBranch}`,
    `Head branch: ${context.headBranch}`,
    `Commit count: ${context.commitCount}`,
    "",
    "Validation notes supplied by the workflow caller:",
    validationNotes || "None supplied.",
    "",
    "Existing pull request title, when regenerating:",
    existingTitle || "None.",
    "",
    "Existing pull request body, when regenerating:",
    existingBody || "None.",
    "",
    "Commits:",
    context.commits || "None.",
    "",
    "Diffstat:",
    context.diffStat || "None.",
    "",
    "Changed files:",
    context.changedFiles || "None.",
    "",
    "Diff:",
    context.diff || "None.",
  ].join("\n");
}

function parseJsonText(text) {
  const trimmed = text.trim();
  const fenced = trimmed.match(/^```(?:json)?\s*([\s\S]*?)\s*```$/i);
  const jsonText = fenced ? fenced[1].trim() : trimmed;

  try {
    return JSON.parse(jsonText);
  } catch {
    fail("Gemini response was not valid JSON.");
  }
}

function extractGeminiText(response) {
  if (typeof response.output_text === "string") {
    return response.output_text;
  }
  if (typeof response.outputText === "string") {
    return response.outputText;
  }

  const stepTexts = [];
  for (const step of response.steps ?? []) {
    const contents = step?.content ?? step?.modelOutput?.content ?? step?.model_output?.content ?? [];
    for (const content of contents) {
      if (typeof content?.text?.text === "string") {
        stepTexts.push(content.text.text);
      } else if (typeof content?.text === "string") {
        stepTexts.push(content.text);
      }
    }
  }
  if (stepTexts.length > 0) {
    return stepTexts.join("\n");
  }

  const candidateTexts = [];
  for (const candidate of response.candidates ?? []) {
    for (const part of candidate?.content?.parts ?? []) {
      if (typeof part.text === "string") {
        candidateTexts.push(part.text);
      }
    }
  }
  if (candidateTexts.length > 0) {
    return candidateTexts.join("\n");
  }

  fail("Gemini response did not include text content.");
}

function readExistingPullRequest() {
  const prNumber = env("PR_NUMBER").trim();
  if (!prNumber) {
    return { title: "", body: "" };
  }

  const repo = env("GITHUB_REPOSITORY").trim();
  if (!repo) {
    return { title: "", body: "" };
  }

  try {
    const raw = execFileSync(
      "gh",
      ["pr", "view", prNumber, "--repo", repo, "--json", "title,body"],
      {
        encoding: "utf8",
        maxBuffer: 2 * 1024 * 1024,
        stdio: ["ignore", "pipe", "ignore"],
      },
    );
    const parsed = JSON.parse(raw);
    return {
      title: String(parsed.title ?? ""),
      body: String(parsed.body ?? ""),
    };
  } catch {
    return { title: "", body: "" };
  }
}

async function generateContent(model, prompt) {
  const apiKey = env("GEMINI_API_KEY").trim();
  if (!apiKey) {
    fail("GEMINI_API_KEY must be configured.");
  }

  const response = await fetch(INTERACTIONS_ENDPOINT, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "x-goog-api-key": apiKey,
    },
    body: JSON.stringify({
      model,
      input: prompt,
      response_format: {
        type: "text",
        mime_type: "application/json",
        schema: {
          type: "object",
          additionalProperties: false,
          required: ["title", "body"],
          properties: {
            title: {
              type: "string",
              description: "Pull request title.",
            },
            body: {
              type: "string",
              description: "Pull request body in Markdown.",
            },
          },
        },
      },
    }),
  });

  if (!response.ok) {
    let details = "";
    try {
      const errorBody = await response.json();
      details = errorBody?.error?.message ? ` ${errorBody.error.message}` : "";
    } catch {
      details = "";
    }
    fail(`Gemini API request failed with HTTP ${response.status}.${details}`);
  }

  return parseJsonText(extractGeminiText(await response.json()));
}

function validateGeneratedContent(content) {
  if (!content || typeof content !== "object") {
    fail("Gemini response must be an object.");
  }

  const title = String(content.title ?? "").trim();
  const body = String(content.body ?? "").trim();
  if (!title || !body) {
    fail("Generated title and body are required.");
  }
  if (/[\r\n]/.test(title)) {
    fail("Generated title must be a single line.");
  }

  const blocklistConfig = readConfig("PR_ATTRIBUTION_BLOCKLIST", { required: true });
  const blocklist = parseLines(blocklistConfig);
  const combined = `${title}\n${body}`;
  for (const entry of blocklist) {
    if (entry.startsWith("regex:")) {
      let pattern;
      try {
        pattern = new RegExp(entry.slice("regex:".length), "i");
      } catch {
        fail("Configured blocklist contains an invalid regex entry.");
      }
      if (pattern.test(combined)) {
        fail("Generated PR content matched a configured blocklist entry.");
      }
    } else if (combined.toLowerCase().includes(entry.toLowerCase())) {
      fail("Generated PR content matched a configured blocklist entry.");
    }
  }

  return { title, body };
}

function writeContentFiles(title, body) {
  const outputDir = env("PR_OUTPUT_DIR") || env("RUNNER_TEMP") || tmpdir();
  mkdirSync(outputDir, { recursive: true });

  const titleFile = join(outputDir, "generated-pr-title.txt");
  const bodyFile = join(outputDir, "generated-pr-body.md");
  writeFileSync(titleFile, `${title}\n`, "utf8");
  writeFileSync(bodyFile, `${body}\n`, "utf8");

  return { titleFile, bodyFile };
}

async function main() {
  const baseBranch = normalizeBranch(
    env("PR_BASE_BRANCH") || readConfig("PR_DEFAULT_BASE_BRANCH") || "develop",
    "base_branch",
  );
  const headBranch = normalizeBranch(env("PR_HEAD_BRANCH"), "head_branch");
  if (baseBranch === headBranch) {
    fail("head_branch must be different from base_branch.");
  }

  const model = resolveModel();
  ensureAllowedModel(model);

  const configuredPrompt = readConfig("PR_GENERATOR_PROMPT", { required: true });
  const context = collectContext(baseBranch, headBranch);
  const prompt = buildPrompt(configuredPrompt, context);
  const generated = await generateContent(model, prompt);
  const { title, body } = validateGeneratedContent(generated);
  const { titleFile, bodyFile } = writeContentFiles(title, body);

  appendOutput("title", title);
  appendOutput("title_file", titleFile);
  appendOutput("body_file", bodyFile);
  appendOutput("model", model);
  appendOutput("base_branch", baseBranch);
  appendOutput("head_branch", headBranch);
  appendSummary([
    "## Pull request generation",
    "",
    `- Model: \`${model}\``,
    `- Base: \`${baseBranch}\``,
    `- Head: \`${headBranch}\``,
    `- Commits: \`${context.commitCount}\``,
  ].join("\n"));
}

main().catch((error) => {
  console.error(`Error: ${error instanceof Error ? error.message : "Unexpected failure."}`);
  process.exit(1);
});
