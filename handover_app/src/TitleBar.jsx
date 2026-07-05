import { getCurrentWindow } from '@tauri-apps/api/window'

const isTauri = Boolean(import.meta.env.TAURI_ENV_PLATFORM)

function WindowButton({ label, onClick, children }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className="flex h-full w-9 items-center justify-center text-[#cccccc] hover:bg-[#3a3a3a] active:bg-[#454545]"
    >
      {children}
    </button>
  )
}

export default function TitleBar() {
  if (!isTauri) return null

  const win = getCurrentWindow()

  return (
    <header className="flex h-6 shrink-0 select-none items-stretch border-b border-[#333333] bg-[#252526] text-[#cccccc]">
      <div
        data-tauri-drag-region
        className="flex min-w-0 flex-1 items-center justify-center gap-1.5 px-2 text-[11px] font-medium tracking-wide"
      >
        <img src="/app-icon.png" alt="" className="size-3.5 shrink-0 rounded-sm" aria-hidden />
        Handover
      </div>
      <div className="flex h-full shrink-0 items-stretch">
        <WindowButton label="Minimize" onClick={() => void win.minimize()}>
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
            <path d="M1 8h8" stroke="currentColor" strokeWidth="1.2" />
          </svg>
        </WindowButton>
        <WindowButton label="Maximize" onClick={() => void win.toggleMaximize()}>
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
            <rect
              x="1.5"
              y="1.5"
              width="7"
              height="7"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.2"
            />
          </svg>
        </WindowButton>
        <WindowButton label="Close" onClick={() => void win.close()}>
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
            <path
              d="M2 2l6 6M8 2L2 8"
              stroke="currentColor"
              strokeWidth="1.2"
              strokeLinecap="round"
            />
          </svg>
        </WindowButton>
      </div>
    </header>
  )
}
