export const meta = {
  name: 'freenpipe-wp-batch',
  description: 'Freenpipe work packages: implement (or fix CI) in worktrees, verification + compliance review with up to two fix rounds, then push and record on a draft PR',
  whenToUse: 'Coordinator sessions (docs/dev/cloud-coordinator.md). args: {base, repo?, session_url, wps: [{id, title, mode?: "impl"|"ci-fix"|"publish", ci_failure?, result?, extra?, extraReview?}]}',
  phases: [
    { title: 'Implement', detail: 'one worker per WP in its own git worktree (resumes a pushed branch)' },
    { title: 'Review', detail: 'verification and compliance reviewers in parallel' },
    { title: 'Fix', detail: 'worker addresses blocker/major findings (max 2 rounds)' },
    { title: 'Publish', detail: 'push the branch, open or update the draft PR, record the reviews' },
  ],
}
// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe
//
// Run with the Workflow tool: {scriptPath: "<repo>/tools/cloud/workflows/wp-batch.js", args: {...}}.
// A workflow runs at most min(16, CPUs - 2) agents at once, so on a 4-CPU VM give each run one or
// two WPs and start several runs side by side. Every durable result lands on GitHub (pushed branch,
// draft PR description, review comment); nothing here depends on this VM surviving.

const BASE = args.base
const REPO = args.repo || '/home/user/Freenpipe'
const SESSION = args.session_url || ''
const WPS = args.wps
const TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>' + (SESSION ? `\nClaude-Session: ${SESSION}` : '')
const WT = id => `/root/.cache/freenpipe/wt/${id}`

const CHECKS = { type: 'array', items: { type: 'object', properties: { command: { type: 'string' }, result: { type: 'string' } }, required: ['command', 'result'] } }
const IMPL_SCHEMA = {
  type: 'object',
  properties: {
    wp: { type: 'string' },
    head_sha: { type: 'string' },
    pushed: { type: 'boolean' },
    complete: { type: 'boolean' },
    summary: { type: 'string' },
    files_changed: { type: 'array', items: { type: 'string' } },
    checks: CHECKS,
    acceptance: { type: 'array', items: { type: 'object', properties: { check: { type: 'string' }, status: { type: 'string', enum: ['pass', 'fail', 'ci', 'smoke', 'not_run'] }, note: { type: 'string' } }, required: ['check', 'status'] } },
    open_questions: { type: 'array', items: { type: 'string' } },
    smoke_entries: { type: 'array', items: { type: 'string' } },
    verbatim_hits: { type: 'array', items: { type: 'string' } },
    follow_ups: { type: 'array', items: { type: 'string' } },
    concerns: { type: 'array', items: { type: 'string' } },
  },
  required: ['wp', 'head_sha', 'complete', 'summary', 'checks', 'acceptance'],
}
const REVIEW_SCHEMA = {
  type: 'object',
  properties: {
    verdict: { type: 'string', enum: ['APPROVE', 'CHANGES'] },
    findings: { type: 'array', items: { type: 'object', properties: { severity: { type: 'string', enum: ['blocker', 'major', 'minor'] }, location: { type: 'string' }, problem: { type: 'string' }, fix: { type: 'string' } }, required: ['severity', 'problem'] } },
    summary: { type: 'string' },
  },
  required: ['verdict', 'findings', 'summary'],
}
const FIX_SCHEMA = {
  type: 'object',
  properties: {
    head_sha: { type: 'string' },
    pushed: { type: 'boolean' },
    responses: { type: 'array', items: { type: 'object', properties: { finding: { type: 'string' }, action: { type: 'string', enum: ['fixed', 'rebutted', 'deferred'] }, note: { type: 'string' } }, required: ['finding', 'action'] } },
    checks: CHECKS,
  },
  required: ['head_sha', 'responses'],
}
const PUBLISH_SCHEMA = {
  type: 'object',
  properties: { pr_number: { type: 'number' }, pr_url: { type: 'string' }, head_sha: { type: 'string' }, notes: { type: 'string' } },
  required: ['pr_number', 'head_sha'],
}

const SHARED_RULES = `Environment rules (several workers share this VM at the same time):
- Each Bash call is a fresh shell: prefix every build/test/git-push command with \`. ~/.freenpipe-env.sh &&\`.
- The cargo target dir ($CARGO_TARGET_DIR) is shared. "Blocking waiting for file lock" means another worker is building: wait, never kill other processes. A first build of a crate in a new worktree can take 10-30 minutes on this 4-core VM: run long builds with run_in_background and poll the log file.
- Disk is limited (about 25 GB shared). Run targeted tests only (\`cargo test -p <crate> --lib <filter>\` or \`--test <name>\`); never \`cargo test --workspace\` or a whole engine integration suite. Do not run fp_disk_check or delete anything under $CARGO_TARGET_DIR; if \`df -h /\` shows under 6 GB free, say so in concerns.
- Never edit ${REPO} (the coordinator's checkout) or other worktrees under $FP_STATE/wt/. Push only your own branch, only with \`python3 tools/cloud/fp.py push\` run inside your worktree. Never push other branches, never force-push, never open or edit PRs yourself.
- Never use git reset --hard, git clean, git checkout -- . or git stash.
- Checkpoint: commit and \`fp.py push\` at least every 30-45 minutes of work and at the end. The VM can disappear (usage limits, idle reclaim); only pushed work survives. If fp.py push prints STOP, another session owns the branch: stop and report it in concerns.`

const commitRule = `Commit with \`git commit -F -\`; every message ends with this exact trailer block:

${TRAILER}`

function implPrompt(w) {
  return `You are a Freenpipe WORKER implementing work package ${w.id} ("${w.title}") in this clean-room fork. A coordinator session reviews, merges and pushes main.

Setup (once): \`cd ${REPO} && . ~/.freenpipe-env.sh && bash tools/cloud/wp-worktree.sh ${w.id} ${BASE}\` prints your worktree, ${WT(w.id)}, on branch wp/${w.id}. If that branch already has commits beyond ${BASE} (\`git -C ${WT(w.id)} log --stat ${BASE}..HEAD\`), an earlier session started this WP: read them and continue from there rather than starting over. For frontend tests run \`bun install\` in the worktree's apps/screenpipe-app-tauri first (bun's cache makes it quick); if it fails, say so.

${SHARED_RULES}

Process: follow docs/dev/cloud-runbook.md sections 1.2-1.5 and 3.2-3.4 (read them in the worktree), skipping fp.py pick/claim/template/open-pr and everything from section 3.5 on (review and PRs are the coordinator's).
1. Read the WP section of docs/clean-room/PLAN.md (grep -n "^#### ${w.id} "), PLAN.md sections 2, 5 and 6, docs/clean-room/DECISIONS.md (binding, wins over PLAN.md) and the spec text for every item id the WP lists (grep -n "<item-id>" docs/clean-room/specs/*.md and read those ranges; the spec files are large, so read ranges, not whole files).
2. Read the code at the owner paths. Edit only the WP's owner paths and regions; the listed files are the plan's best guess, so small necessary edits elsewhere are fine when you justify them in the summary.
3. Implement the whole WP to production quality: no TODO stubs where the plan asks for behaviour. Write original code and prose. Add tests proving the acceptance checks wherever they can run on Linux. Keep cfg(windows) code compiling in principle and keep pure logic outside cfg blocks so Linux tests cover it.
4. Run the checks from runbook 3.4 for what you touched: cargo check -p <crate> --tests, targeted cargo test, the app crate check if touched (cargo check --manifest-path apps/screenpipe-app-tauri/src-tauri/Cargo.toml --tests), bun test for touched frontend code, \`bun test scripts/ci/vendor-guard.test.ts\`, and the clean-room gate \`bash scripts/ci/verbatim-diff.sh ${BASE} HEAD\`. Two checks CI always runs and earlier workers kept missing:
   - If you add, remove or change any \`#[tauri::command]\` (or a type one uses), run \`cd apps/screenpipe-app-tauri && bun run bindings:generate\` and commit the regenerated lib/utils/tauri.ts (never edit it by hand); \`bun run bindings:check\` must pass. It builds the app crate, so start it in the background early.
   - If you touch anything under apps/screenpipe-app-tauri that the frontend compiles, run \`bun run build\` there (Next static export with a full type check); it must pass.
5. Acceptance checks that need Windows hardware or a desktop: add entries to docs/dev/windows-smoke-checklist.md (format in that file, under a "## ${w.id}" heading in plan order); never tick them. Spec ambiguities you decide: append to docs/clean-room/OPEN-QUESTIONS.md under "## ${w.id}" (question, decision, reason; create the file with a "# Open questions" heading if missing).
6. ${commitRule}

Finish with everything committed and pushed (fp.py push) and the worktree on branch wp/${w.id}.

Return: head_sha (git rev-parse HEAD in the worktree), pushed, complete (true only if every part of the WP is implemented), a summary suitable for a PR description (3-8 bullets), files changed, every check command with its result, each acceptance check with status pass|fail|ci (left to the CI windows/linux job)|smoke (checklist entry added)|not_run, open questions recorded, smoke checklist entries added, verbatim-gate hits with justification, follow-ups for later WPs, and concerns for the coordinator (for example files other WPs of the same wave also edit).
${w.extra || ''}`
}

function ciFixPrompt(w) {
  return `You are the Freenpipe WORKER for ${w.id} ("${w.title}"). CI failed on its pull request; make it green. Worktree: \`cd ${REPO} && . ~/.freenpipe-env.sh && bash tools/cloud/wp-worktree.sh ${w.id} ${BASE}\` prints ${WT(w.id)} (branch wp/${w.id}, already pushed).

${SHARED_RULES}

The coordinator's notes on the failure:
${w.ci_failure || '(none; find it yourself)'}

Read the failing job logs yourself: \`python3 tools/cloud/fp.py ci <PR> --why\` names the failed jobs and steps; read logs with the GitHub MCP tool mcp__github__get_job_logs (load it with ToolSearch "select:mcp__github__get_job_logs"; owner normiecore, repo Freenpipe, return_content true, tail_lines 400 or more), because direct log downloads are blocked here. Root-cause the failure. Windows-only code cannot be built here: read it carefully and make the fix obviously right. Follow docs/dev/cloud-runbook.md section 3.9/3.10 rules; never skip, disable or quarantine a test to get green. If the failure is not this branch's (it is red on main too), say so with evidence instead of changing code. ${commitRule}

Finish with everything committed and pushed (fp.py push). Return head_sha, pushed, one response per root cause found (finding = the failure, action fixed|rebutted|deferred, note) and the checks you ran.
${w.extra || ''}`
}

const ROLES = [
  { key: 'verification', name: 'Verification reviewer' },
  { key: 'compliance', name: 'Compliance reviewer' },
]

function reviewPrompt(w, role, sha, round, prev) {
  const re = !prev ? '' : `
This is a re-review. You previously reviewed head ${prev.sha}; the worker has since committed fixes. Focus on the commits since then (git diff ${prev.sha}..${sha}) and on whether your earlier findings are resolved, but re-check anything those commits touch. Your earlier findings were:
${JSON.stringify(prev.findings, null, 1)}
The worker's responses (a rebuttal with evidence can be accepted):
${JSON.stringify(prev.responses || [], null, 1)}`
  return `You review Freenpipe work package ${w.id} ("${w.title}") on branch wp/${w.id}, head ${sha}, in the worktree ${WT(w.id)}. The base commit is ${BASE}. Before any build or test command run \`. ~/.freenpipe-env.sh\`. First confirm \`git -C ${WT(w.id)} rev-parse HEAD\` equals ${sha}; if it does not, report that as a blocker and stop.

You are the ${role.name}. Read docs/dev/cloud-reviewers.md: follow the shared preamble and the "${role.name}" brief exactly, reading "origin/main" as ${BASE} (so the change is \`git diff ${BASE}...${sha}\` and \`git log --stat ${BASE}..${sha}\`, and the gate is \`bash scripts/ci/verbatim-diff.sh ${BASE} ${sha}\`). Read the WP section of docs/clean-room/PLAN.md, its sections 2, 5 and 6, docs/clean-room/DECISIONS.md, the spec text for each item id (grep the ids in docs/clean-room/specs/*.md) and CLAUDE.md.
${re}
Do not modify tracked files, commit, push or switch branches in that worktree; another reviewer may be using it at the same time. Scratch files go in $FP_STATE/review/${w.id}/ (you may add a detached worktree there if you need to experiment, and remove it when done). The cargo target dir is shared: a "Blocking waiting for file lock" message means another build is running, so wait. Run targeted tests only (no workspace-wide test runs; disk is limited). The VM is Linux: judge cfg(windows) code by reading and say what CI's windows job must prove.

Request CHANGES only for blocker or major findings; minor findings still allow APPROVE. Return verdict, findings (severity, location as file:line, problem, fix) and a 3-6 line summary of what you checked and ran, with results.
${role.key === 'verification' ? `
CI runs these on every PR, so check them yourself: if the change touches any \`#[tauri::command]\` or a type one uses, \`cd apps/screenpipe-app-tauri && bun run bindings:check\` must pass (stale lib/utils/tauri.ts is a blocker); if it touches frontend code, \`bun run build\` in apps/screenpipe-app-tauri must pass (type errors are blockers).` : ''}
${w.extraReview || ''}`
}

function fixPrompt(w, head, round, findings) {
  return `You are the Freenpipe WORKER for ${w.id} ("${w.title}"), now fixing review findings (round ${round}) on branch wp/${w.id} in the worktree ${WT(w.id)} (current head ${head}, base ${BASE}). Read the WP section of docs/clean-room/PLAN.md and docs/dev/cloud-runbook.md sections 1.2-1.5 and 3.3-3.4 again if you need them; the same rules apply.

${SHARED_RULES}

Findings from the two independent reviewers:
${JSON.stringify(findings, null, 1)}

Fix every blocker and major finding. When a finding is wrong, rebut it with concrete evidence (file:line, command output) instead of changing code. Fix minor findings when the fix is cheap and clearly correct. Re-run the checks your changes affect (cargo check/test for touched crates, bun test, vendor guard, \`bash scripts/ci/verbatim-diff.sh ${BASE} HEAD\`). ${commitRule}

Finish with everything committed and pushed (fp.py push) on branch wp/${w.id}. Return head_sha, pushed, one response per finding (fixed|rebutted|deferred, with a note) and the checks you ran.
${w.extra || ''}`
}

function publishPrompt(w, res) {
  const approved = res.status === 'approved'
  return `You publish the result of Freenpipe work package ${w.id} ("${w.title}") to GitHub. Worktree ${WT(w.id)}, branch wp/${w.id}, expected head ${res.head}. Run every command from inside the worktree after \`. ~/.freenpipe-env.sh &&\`.

1. Confirm \`git rev-parse HEAD\` is ${res.head} and the tree is clean, then \`python3 tools/cloud/fp.py push\` (it must say pushed; if it prints STOP, report that and do nothing else).
2. Write the PR description: \`python3 tools/cloud/fp.py template ${w.id} --worker coordinator > $FP_STATE/pr-${w.id}.md\`, then fill in every section from the data below, in plain factual prose (no marketing words, no emoji). "What changed": the summary bullets. "Acceptance checks": tick [x] only for pass, and say where each ran or will run (VM Linux, CI windows job, CI linux job, smoke checklist entry). "Tests run": the check commands and results. "Verbatim gate": hits and justifications or "0 hits". "Windows hardware smoke items": the entries or "none". "Review": exactly two bullets, \`- Verification reviewer (head \\\`${res.head}\\\`): <VERDICT> - <summary>\` and the same for the Compliance reviewer, using the final round's verdicts${approved ? '' : ' (state plainly that changes are still open)'}. "Spec decisions and open questions" and "Follow-ups": from the data, or "none". Keep the \`<!-- fp-target: ${w.id} -->\` line. End the file with a blank line, then these two lines:
🤖 Generated with [Claude Code](https://claude.com/claude-code)
${SESSION}
3. \`python3 tools/cloud/fp.py open-pr ${w.id} --draft --update-body --body-file $FP_STATE/pr-${w.id}.md\` and note the PR number it prints.
4. Write $FP_STATE/review-${w.id}.md: a heading "Review record for ${w.id} at ${res.head}", then for every round the head, each reviewer's verdict, summary and findings (severity, location, problem, fix) and the worker's responses. Post it with \`python3 tools/cloud/fp.py comment <PR> --body-file $FP_STATE/review-${w.id}.md\`.${approved ? '' : `
5. The reviews still request changes, so also run \`python3 tools/cloud/fp.py block <PR> --reason "review changes still open after two fix rounds; see the review record"\`.`}

Data (JSON):
${JSON.stringify({ implementation: res.impl, ci_fix: res.ciFix, rounds: res.rounds, status: res.status }, null, 1)}

Return pr_number, pr_url, head_sha (what was pushed) and notes on anything that went wrong.`
}

async function runWP(w) {
  const mode = w.mode || 'impl'
  if (mode === 'publish') {   // w.result: a finished runWP-shaped result whose publish step never ran
    const res = { ...w.result, wp: w.id }
    res.pr = await agent(publishPrompt(w, res), { label: `publish:${w.id}`, phase: 'Publish', schema: PUBLISH_SCHEMA, effort: 'low' })
    return res
  }
  let impl = null, ciFix = null, head
  if (mode === 'ci-fix') {
    ciFix = await agent(ciFixPrompt(w), { label: `ci-fix:${w.id}`, phase: 'Fix', schema: FIX_SCHEMA })
    if (!ciFix) return { wp: w.id, status: 'ci_fix_failed' }
    head = ciFix.head_sha
  } else {
    impl = await agent(implPrompt(w), { label: `impl:${w.id}`, phase: 'Implement', schema: IMPL_SCHEMA })
    if (!impl) return { wp: w.id, status: 'impl_failed' }
    head = impl.head_sha
  }
  log(`${w.id}: ${mode} done at ${head.slice(0, 8)}${impl ? ` (complete=${impl.complete})` : ''}`)
  const rounds = []
  const prevByRole = {}
  let approved = false
  for (let round = 0; round <= 2; round++) {
    const reviews = await parallel(ROLES.map(role => () =>
      agent(reviewPrompt(w, role, head, round, prevByRole[role.key]), {
        label: `review-${role.key}:${w.id}:r${round}`, phase: 'Review', schema: REVIEW_SCHEMA,
      }).then(r => (r ? { role: role.key, ...r } : null))))
    const got = reviews.filter(Boolean)
    rounds.push({ round, head, reviews: got })
    log(`${w.id} r${round}: ${got.map(r => `${r.role}=${r.verdict}`).join(', ') || 'no reviews (agents failed)'}`)
    if (got.length === ROLES.length && got.every(r => r.verdict === 'APPROVE')) { approved = true; break }
    if (got.length < ROLES.length || round === 2) break
    const findings = got.flatMap(r => r.findings.map(f => ({ reviewer: r.role, ...f })))
    const fix = await agent(fixPrompt(w, head, round + 1, findings), { label: `fix:${w.id}:r${round + 1}`, phase: 'Fix', schema: FIX_SCHEMA })
    if (!fix) break
    for (const r of got) prevByRole[r.role] = { sha: head, findings: r.findings, responses: fix.responses }
    rounds[rounds.length - 1].fix = fix
    head = fix.head_sha
  }
  const reviewsComplete = rounds.length > 0 && rounds[rounds.length - 1].reviews.length === ROLES.length
  const res = { wp: w.id, status: approved ? 'approved' : (reviewsComplete ? 'changes_open' : 'review_incomplete'), head, impl, ciFix, rounds }
  if (res.status === 'review_incomplete') return res   // agents failed (usage limit?): branch is pushed, rerun later
  const pub = await agent(publishPrompt(w, res), { label: `publish:${w.id}`, phase: 'Publish', schema: PUBLISH_SCHEMA, effort: 'low' })
  res.pr = pub
  log(`${w.id}: ${res.status}${pub ? `, PR #${pub.pr_number}` : ', publish failed'}`)
  return res
}

return await parallel(WPS.map(w => () => runWP(w)))
