/**
 * Turns any handoff response into `{ ok, kind, message }`.
 *
 * The backend answers every handoff, success or failure, with
 * `{ status: 'ok' | 'error', kind, message }`; request validation errors
 * (unknown project or instance) carry only `detail`.
 * `kind` is one of: git_committed | git_no_changes | summary_written |
 * not_git_repo | git_failed | summary_timed_out | source_not_running |
 * target_not_running | internal.
 */
export function parseHandoffResult(body) {
  const kind = body?.kind ?? null
  if (body?.status === 'ok') {
    return { ok: true, kind, message: body.message ?? 'Handover complete' }
  }
  const message =
    body?.message ??
    (typeof body?.detail === 'string' ? body.detail : null) ??
    'Handoff failed'
  return { ok: false, kind, message }
}
