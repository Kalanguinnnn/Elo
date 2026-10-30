// Elo identity: two rounded brackets (the link) around a sage dot (the
// checkpoint). Colors come from CSS variables so the mark follows the theme.

export function EloMark({ size = 22, title }: { size?: number; title?: string }) {
  return (
    <svg width={size} height={(size * 108) / 136} viewBox="-10 -10 156 128" role="img" aria-label={title ?? "Elo"}>
      <g fill="none" stroke="var(--mark)" strokeWidth="20" strokeLinecap="round" strokeLinejoin="round">
        <path d="M64 10H30A20 20 0 0 0 10 30V50" />
        <path d="M72 98H106A20 20 0 0 0 126 78V58" />
      </g>
      <circle cx="68" cy="54" r="16" fill="var(--sage)" />
    </svg>
  );
}

export function EloLogo({ size = 22 }: { size?: number }) {
  return (
    <span className="logo" style={{ fontSize: size }}>
      <EloMark size={size * 1.25} />
      <span className="wordmark">elo</span>
    </span>
  );
}

/** Small status light, as in the menu-bar mockup ("Tudo em dia"). */
export function Dot({ tone = "ok" }: { tone?: "ok" | "warn" | "err" | "info" | "idle" }) {
  return <span className={`dot ${tone}`} aria-hidden />;
}
