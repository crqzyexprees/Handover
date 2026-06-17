import { useCallback, useEffect, useState } from 'react'
import * as api from './api.js'

function formatTimestamp(iso) {
  if (!iso) return 'Unknown time'
  try {
    return new Date(iso).toLocaleString()
  } catch {
    return iso
  }
}

function downloadText(filename, content) {
  const blob = new Blob([content], { type: 'text/markdown;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  link.click()
  URL.revokeObjectURL(url)
}

const selectClassName =
  'rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-xs text-[#cccccc]'

export default function HandoffHistoryPanel({
  open,
  projectId,
  projectName = 'project',
  onClose,
}) {
  const [files, setFiles] = useState([])
  const [selectedFilename, setSelectedFilename] = useState(null)
  const [preview, setPreview] = useState('')
  const [loadingList, setLoadingList] = useState(false)
  const [loadingPreview, setLoadingPreview] = useState(false)
  const [exporting, setExporting] = useState(false)
  const [errorText, setErrorText] = useState('')

  const [viewMode, setViewMode] = useState('preview')
  const [diffTab, setDiffTab] = useState('handoff')
  const [fromFile, setFromFile] = useState('')
  const [toFile, setToFile] = useState('')
  const [gitFromRef, setGitFromRef] = useState('HEAD~1')
  const [gitToRef, setGitToRef] = useState('HEAD')
  const [diffText, setDiffText] = useState('')
  const [loadingDiff, setLoadingDiff] = useState(false)

  const loadList = useCallback(async () => {
    if (projectId == null) return
    setLoadingList(true)
    setErrorText('')
    const { data, error } = await api.listHandoffFiles(projectId)
    setLoadingList(false)
    if (error) {
      setErrorText('Could not load handoff history.')
      setFiles([])
      return
    }
    const nextFiles = Array.isArray(data?.files) ? data.files : []
    setFiles(nextFiles)
    if (nextFiles.length > 0) {
      const preferred =
        nextFiles.find((f) => f.is_latest)?.filename ?? nextFiles[0].filename
      setSelectedFilename(preferred)
      if (nextFiles.length >= 2) {
        setFromFile(nextFiles[1].filename)
        setToFile(nextFiles[0].filename)
      } else {
        setFromFile(nextFiles[0].filename)
        setToFile(nextFiles[0].filename)
      }
    } else {
      setSelectedFilename(null)
      setPreview('')
      setFromFile('')
      setToFile('')
    }
  }, [projectId])

  useEffect(() => {
    if (!open || projectId == null) return
    const timer = window.setTimeout(() => {
      void loadList()
    }, 0)
    return () => window.clearTimeout(timer)
  }, [open, projectId, loadList])

  useEffect(() => {
    if (!open || projectId == null || selectedFilename == null || viewMode !== 'preview') {
      if (viewMode !== 'preview') return
      const timer = window.setTimeout(() => {
        setPreview('')
      }, 0)
      return () => window.clearTimeout(timer)
    }
    let cancelled = false
    const timer = window.setTimeout(() => {
      setLoadingPreview(true)
      void api.getHandoffFile(projectId, selectedFilename).then(({ data, error }) => {
        if (cancelled) return
        setLoadingPreview(false)
        if (error) {
          setPreview('')
          setErrorText('Could not load handoff preview.')
          return
        }
        setPreview(data?.content ?? '')
      })
    }, 0)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [open, projectId, selectedFilename, viewMode])

  const loadDiff = useCallback(async () => {
    if (projectId == null) return
    setLoadingDiff(true)
    setErrorText('')
    setDiffText('')

    if (diffTab === 'handoff') {
      if (!fromFile || !toFile) {
        setLoadingDiff(false)
        setErrorText('Select from and to handoff files.')
        return
      }
      const { data, error } = await api.diffHandoffFiles(projectId, fromFile, toFile)
      setLoadingDiff(false)
      if (error) {
        setErrorText('Could not load handoff diff.')
        return
      }
      setDiffText(data?.diff ?? '')
      return
    }

    const fromRef = gitFromRef.trim()
    const toRef = gitToRef.trim()
    if (!fromRef || !toRef) {
      setLoadingDiff(false)
      setErrorText('Enter git from and to refs.')
      return
    }
    const { data, error } = await api.gitDiff(projectId, fromRef, toRef)
    setLoadingDiff(false)
    if (error) {
      const detail =
        typeof error.detail === 'string'
          ? error.detail
          : 'Could not load git diff.'
      setErrorText(detail)
      return
    }
    setDiffText(data?.diff ?? '')
  }, [diffTab, fromFile, gitFromRef, gitToRef, projectId, toFile])

  useEffect(() => {
    if (!open || projectId == null || viewMode !== 'compare') return
    const timer = window.setTimeout(() => {
      void loadDiff()
    }, 0)
    return () => window.clearTimeout(timer)
  }, [open, projectId, viewMode, loadDiff])

  const handleExport = async () => {
    if (projectId == null) return
    setExporting(true)
    setErrorText('')
    const { data, error } = await api.exportHandoffLog(projectId)
    setExporting(false)
    if (error || typeof data !== 'string') {
      setErrorText('Could not export handoff log.')
      return
    }
    const safeName = projectName.replace(/[^\w.-]+/g, '_') || 'project'
    downloadText(`${safeName}-handoff-log.md`, data)
  }

  if (!open) return null

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="presentation"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        className="flex h-[min(720px,90vh)] w-full max-w-4xl flex-col rounded-md border border-[#333333] bg-[#252526] text-[#cccccc] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex shrink-0 items-center justify-between border-b border-[#333333] px-4 py-3">
          <div>
            <h2 className="text-sm font-semibold">Handoff History</h2>
            <p className="mt-0.5 text-xs text-[#808080]">
              Timeline from `.handover/handoffs/` — preview or compare versions
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => void loadList()}
              disabled={loadingList}
              className="rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-xs hover:bg-[#2a2d2e] disabled:opacity-40"
            >
              Refresh
            </button>
            <button
              type="button"
              onClick={() => void handleExport()}
              disabled={exporting || files.length === 0}
              className="rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-xs hover:bg-[#2a2d2e] disabled:opacity-40"
            >
              {exporting ? 'Exporting…' : 'Export log'}
            </button>
            <button
              type="button"
              onClick={onClose}
              className="rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-xs hover:bg-[#2a2d2e]"
            >
              Close
            </button>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-2 border-b border-[#333333] px-4 py-2">
          <button
            type="button"
            onClick={() => setViewMode('preview')}
            className={`rounded px-2 py-1 text-xs ${
              viewMode === 'preview'
                ? 'bg-[#37373d] text-[#cccccc]'
                : 'text-[#808080] hover:bg-[#2a2d2e]'
            }`}
          >
            Preview
          </button>
          <button
            type="button"
            onClick={() => setViewMode('compare')}
            disabled={files.length === 0}
            className={`rounded px-2 py-1 text-xs disabled:opacity-40 ${
              viewMode === 'compare'
                ? 'bg-[#37373d] text-[#cccccc]'
                : 'text-[#808080] hover:bg-[#2a2d2e]'
            }`}
          >
            Compare
          </button>
        </div>

        <div className="flex min-h-0 flex-1">
          {viewMode === 'preview' ? (
            <>
              <aside className="w-56 shrink-0 overflow-y-auto border-r border-[#333333] p-2">
                {loadingList ? (
                  <p className="px-2 py-3 text-xs text-[#808080]">Loading…</p>
                ) : files.length === 0 ? (
                  <p className="px-2 py-3 text-xs leading-relaxed text-[#808080]">
                    No handoff files yet. Run a Summary handoff to create `latest.md`.
                  </p>
                ) : (
                  <ul className="space-y-1">
                    {files.map((file) => {
                      const selected = file.filename === selectedFilename
                      return (
                        <li key={file.filename}>
                          <button
                            type="button"
                            onClick={() => setSelectedFilename(file.filename)}
                            className={`w-full rounded px-2 py-2 text-left text-xs ${
                              selected
                                ? 'bg-[#37373d] text-[#cccccc]'
                                : 'text-[#aaaaaa] hover:bg-[#2a2d2e]'
                            }`}
                          >
                            <div className="font-medium">
                              {file.is_latest ? 'latest.md' : file.filename}
                            </div>
                            <div className="mt-0.5 text-[10px] text-[#808080]">
                              {formatTimestamp(file.modified)}
                            </div>
                            <div className="text-[10px] text-[#666666]">
                              {file.size_bytes} bytes
                            </div>
                          </button>
                        </li>
                      )
                    })}
                  </ul>
                )}
              </aside>

              <section className="flex min-w-0 flex-1 flex-col">
                <div className="shrink-0 border-b border-[#333333] px-4 py-2 text-xs text-[#808080]">
                  {selectedFilename ? `Preview: ${selectedFilename}` : 'Select a handoff file'}
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto p-4">
                  {loadingPreview ? (
                    <p className="text-sm text-[#808080]">Loading preview…</p>
                  ) : preview ? (
                    <pre className="whitespace-pre-wrap font-mono text-xs leading-relaxed text-[#d4d4d4]">
                      {preview}
                    </pre>
                  ) : (
                    <p className="text-sm text-[#808080]">
                      {files.length === 0
                        ? 'Handoff previews will appear here.'
                        : 'Select a file from the timeline.'}
                    </p>
                  )}
                </div>
              </section>
            </>
          ) : (
            <section className="flex min-w-0 flex-1 flex-col">
              <div className="shrink-0 space-y-3 border-b border-[#333333] px-4 py-3">
                <div className="flex gap-2">
                  <button
                    type="button"
                    onClick={() => setDiffTab('handoff')}
                    className={`rounded px-2 py-1 text-xs ${
                      diffTab === 'handoff'
                        ? 'bg-[#37373d] text-[#cccccc]'
                        : 'text-[#808080] hover:bg-[#2a2d2e]'
                    }`}
                  >
                    Handoff files
                  </button>
                  <button
                    type="button"
                    onClick={() => setDiffTab('git')}
                    className={`rounded px-2 py-1 text-xs ${
                      diffTab === 'git'
                        ? 'bg-[#37373d] text-[#cccccc]'
                        : 'text-[#808080] hover:bg-[#2a2d2e]'
                    }`}
                  >
                    Git diff
                  </button>
                </div>

                {diffTab === 'handoff' ? (
                  <div className="flex flex-wrap items-center gap-2 text-xs">
                    <label className="flex items-center gap-1 text-[#808080]">
                      From
                      <select
                        value={fromFile}
                        onChange={(e) => setFromFile(e.target.value)}
                        className={selectClassName}
                      >
                        {files.map((file) => (
                          <option key={`from-${file.filename}`} value={file.filename}>
                            {file.filename}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label className="flex items-center gap-1 text-[#808080]">
                      To
                      <select
                        value={toFile}
                        onChange={(e) => setToFile(e.target.value)}
                        className={selectClassName}
                      >
                        {files.map((file) => (
                          <option key={`to-${file.filename}`} value={file.filename}>
                            {file.filename}
                          </option>
                        ))}
                      </select>
                    </label>
                    <button
                      type="button"
                      onClick={() => void loadDiff()}
                      disabled={loadingDiff}
                      className="rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-xs hover:bg-[#2a2d2e] disabled:opacity-40"
                    >
                      {loadingDiff ? 'Loading…' : 'Compare'}
                    </button>
                  </div>
                ) : (
                  <div className="flex flex-wrap items-center gap-2 text-xs">
                    <label className="flex items-center gap-1 text-[#808080]">
                      From
                      <input
                        type="text"
                        value={gitFromRef}
                        onChange={(e) => setGitFromRef(e.target.value)}
                        placeholder="HEAD~1"
                        className={`${selectClassName} w-28`}
                      />
                    </label>
                    <label className="flex items-center gap-1 text-[#808080]">
                      To
                      <input
                        type="text"
                        value={gitToRef}
                        onChange={(e) => setGitToRef(e.target.value)}
                        placeholder="HEAD"
                        className={`${selectClassName} w-28`}
                      />
                    </label>
                    <button
                      type="button"
                      onClick={() => void loadDiff()}
                      disabled={loadingDiff}
                      className="rounded border border-[#333333] bg-[#1e1e1e] px-2 py-1 text-xs hover:bg-[#2a2d2e] disabled:opacity-40"
                    >
                      {loadingDiff ? 'Loading…' : 'Compare'}
                    </button>
                  </div>
                )}
              </div>

              <div className="min-h-0 flex-1 overflow-y-auto p-4">
                {loadingDiff ? (
                  <p className="text-sm text-[#808080]">Loading diff…</p>
                ) : diffText ? (
                  <pre className="whitespace-pre-wrap font-mono text-xs leading-relaxed text-[#d4d4d4]">
                    {diffText}
                  </pre>
                ) : (
                  <p className="text-sm text-[#808080]">
                    {diffTab === 'git'
                      ? 'Unified git diff will appear here.'
                      : 'Select handoff files and compare.'}
                  </p>
                )}
              </div>
            </section>
          )}
        </div>

        {errorText ? (
          <p className="shrink-0 border-t border-[#333333] px-4 py-2 text-xs text-red-300">
            {errorText}
          </p>
        ) : null}
      </div>
    </div>
  )
}
