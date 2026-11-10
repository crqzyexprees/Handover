import { describe, expect, it } from 'vitest'
import { parseHandoffResult } from './handoffResult.js'

describe('parseHandoffResult', () => {
  it('reads a success', () => {
    expect(
      parseHandoffResult({ status: 'ok', kind: 'git_committed', message: 'Handoff complete' }),
    ).toEqual({ ok: true, kind: 'git_committed', message: 'Handoff complete' })
  })

  it('reads a flow failure (HTTP 4xx body)', () => {
    expect(
      parseHandoffResult({
        httpStatus: 400,
        status: 'error',
        kind: 'target_not_running',
        message: 'to_instance does not have an active terminal session',
      }),
    ).toEqual({
      ok: false,
      kind: 'target_not_running',
      message: 'to_instance does not have an active terminal session',
    })
  })

  it('falls back to detail for request validation errors', () => {
    expect(parseHandoffResult({ httpStatus: 404, detail: 'from_instance not found' })).toEqual({
      ok: false,
      kind: null,
      message: 'from_instance not found',
    })
  })

  it('handles a network error with no body', () => {
    expect(parseHandoffResult(new Error('Network Error'))).toEqual({
      ok: false,
      kind: null,
      message: 'Network Error',
    })
  })
})
