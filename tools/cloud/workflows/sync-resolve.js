export const meta = {
  name: 'handvel-sync-resolve',
  description: 'Merge origin/main into an approved WP branch, resolve conflicts keeping both sides intent, push, then two independent resolution reviews',
  whenToUse: 'Coordinator (docs/dev/cloud-coordinator.md): an approved, green WP PR conflicts with main in logic, not just append-only docs. args: {wp, title, pr, session_url, other_wps: "what the conflicting merged WPs did"}',
  phases: [
    { title: 'Resolve', detail: 'merge origin/main into the WP branch and resolve conflicts' },
    { title: 'Review', detail: 'verification and compliance reviewers on the resolution' },
    { title: 'Fix', detail: 'one fix round if a reviewer requests changes' },
  ],
}
// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

const { wp, title, pr, session_url, other_wps } = args
const WT = `/root/.cache/freenpipe/wt/${wp}`
const TRAILER = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>' + (session_url ? `\nClaude-Session: ${session_url}` : '')

const RESOLVE = {
  type: 'object',
  properties: {
    merge_sha: { type: 'string' },
    head_sha: { type: 'string' },
    pushed: { type: 'boolean' },
    conflicts: { type: 'array', items: { type: 'object', properties: { file: { type: 'string' }, resolution: { type: 'string' } }, required: ['file', 'resolution'] } },
    checks: { type: 'array', items: { type: 'object', properties: { command: { type: 'string' }, result: { type: 'string' } }, required: ['command', 'result'] } },
    concerns: { type: 'array', items: { type: 'string' } },
  },
  required: ['head_sha', 'conflicts', 'checks'],
}
const REVIEW = {
  type: 'object',
  properties: {
    verdict: { type: 'string', enum: ['APPROVE', 'CHANGES'] },
    findings: { type: 'array', items: { type: 'object', properties: { severity: { type: 'string', enum: ['blocker', 'major', 'minor'] }, location: { type: 'string' }, problem: { type: 'string' }, fix: { type: 'string' } }, required: ['severity', 'problem'] } },
    summary: { type: 'string' },
  },
  required: ['verdict', 'findings', 'summary'],
}

const RULES = `Rules: run \`. ~/.freenpipe-env.sh\` before builds; the cargo target dir is shared, so wait on its lock. Never use git reset --hard, git clean, git checkout -- ., git stash, rebase or force-push. Never edit /home/user/Handvel or other worktrees. Push only with \`python3 tools/cloud/fp.py push\` from inside ${WT}. After a passing \`bun run build\`, delete ${WT}/apps/screenpipe-app-tauri/.next (disk is small). Commit messages end with:

${TRAILER}`

function resolvePrompt(fixFindings) {
  if (fixFindings) {
    return `You resolved a merge of origin/main into wp/${wp} ("${title}") in ${WT}. Reviewers found problems in the resolution:
${JSON.stringify(fixFindings, null, 1)}
Fix every blocker and major finding (rebut wrong ones with evidence), re-run the checks they touch, commit and push. ${RULES}
Return merge_sha (unchanged), head_sha, pushed, conflicts (empty), checks and concerns.`
  }
  return `You are the Handvel merger resolving a conflict. Work package ${wp} ("${title}", PR #${pr}) was approved and CI-green, but main has moved and the branch now conflicts with it. Worktree: ${WT} (branch wp/${wp}).

What the WPs that merged into main since then did, in the conflicting areas:
${other_wps}

Steps:
1. \`cd ${WT} && git status\` must be clean. \`git fetch origin main && git merge --no-ff origin/main\`.
2. For every conflicted file read both sides (\`git diff\`, \`git log --oneline -3 origin/main -- <file>\`, \`git log --oneline -5 HEAD -- <file>\`, and the WP sections in docs/clean-room/PLAN.md) and resolve it so BOTH sides' intent survives: this WP's change and the merged WPs' changes. Do not drop either side's behaviour, and add nothing beyond what the two sides need to fit together. Append-only docs (docs/clean-room/OPEN-QUESTIONS.md, docs/dev/windows-smoke-checklist.md) keep both sides' sections in plan order. Generated files are regenerated, never hand-merged: lib/utils/tauri.ts with \`bun run bindings:generate\`, Cargo.lock with cargo, bun.lock with \`bun install\`.
3. Check the merged tree: in apps/screenpipe-app-tauri run \`bunx tsc --noEmit\`, \`bun run test\` (or the test files covering the touched components) and \`bun run build\`; for touched Rust crates \`cargo check -p <crate> --tests\` and their targeted tests; \`bun test scripts/ci/vendor-guard.test.ts\`.
4. Commit the merge (\`git commit -F -\`, subject "Merge origin/main into wp/${wp}", a body listing each conflict and how it was resolved) and push.
${RULES}
Return merge_sha (the merge commit), head_sha, pushed, every conflict with its resolution, the checks with results, and concerns.`
}

function reviewPrompt(role, res, prev) {
  return `You review only a merge-conflict resolution for Handvel work package ${wp} ("${title}"): merge commit ${res.merge_sha} (branch head ${res.head_sha}) in the worktree ${WT}. Read docs/dev/cloud-reviewers.md and act as the ${role} using its "Merge resolution variant" section. Write the resolution diff with \`git -C ${WT} show --remerge-diff ${res.merge_sha}\` (and \`git -C ${WT} diff ${res.merge_sha}..${res.head_sha}\` for any follow-up commits) and judge whether both sides' intent survived and nothing outside both sides' changes was added. The merged WPs did this in the conflicting areas:
${other_wps}
The resolver reported:
${JSON.stringify(res.conflicts, null, 1)}
Checks it ran:
${JSON.stringify(res.checks, null, 1)}
${prev ? `This is a re-review after fixes; your earlier findings: ${JSON.stringify(prev)}` : ''}
Do not modify tracked files, commit or push. You may run read-only checks (bunx tsc --noEmit, bun test on specific files) in ${WT}. Request CHANGES only for blocker or major findings. Return verdict, findings and a 2-5 line summary.`
}

const ROLES = ['Verification reviewer', 'Compliance reviewer']
let res = await agent(resolvePrompt(null), { label: `resolve:${wp}`, phase: 'Resolve', schema: RESOLVE })
if (!res) return { wp, status: 'resolve_failed' }
const rounds = []
for (let round = 0; round <= 1; round++) {
  const reviews = (await parallel(ROLES.map((role, i) => () => agent(reviewPrompt(role, res, round ? rounds[round - 1].reviews[i]?.findings : null), {
    label: `resolution-${role.split(' ')[0].toLowerCase()}:${wp}:r${round}`, phase: 'Review', schema: REVIEW,
  }).then(r => r ? { role, ...r } : null)))).filter(Boolean)
  rounds.push({ round, head: res.head_sha, reviews })
  if (reviews.length === 2 && reviews.every(r => r.verdict === 'APPROVE')) return { wp, status: 'approved', res, rounds }
  if (reviews.length < 2 || round === 1) break
  const fixed = await agent(resolvePrompt(reviews.flatMap(r => r.findings.map(f => ({ reviewer: r.role, ...f })))), { label: `fix-resolution:${wp}`, phase: 'Fix', schema: RESOLVE })
  if (!fixed) break
  res = { ...res, head_sha: fixed.head_sha, checks: fixed.checks }
}
return { wp, status: 'changes_open', res, rounds }
