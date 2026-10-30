import { useCallback, useEffect, useRef, useState } from "react";
import { sideBySide } from "./diff";
import { errorText } from "./format";
import type { FileView } from "./api";

/** Load data, refresh on an interval, expose errors instead of hiding them. */
export function useLoad<T>(fn: () => Promise<T>, deps: unknown[], intervalMs = 0) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const fnRef = useRef(fn);
  fnRef.current = fn;
  const reload = useCallback(async () => {
    try {
      setData(await fnRef.current());
      setError(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => {
    setLoading(true);
    reload();
    if (!intervalMs) return;
    const t = setInterval(reload, intervalMs);
    return () => clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return { data, error, loading, reload };
}

/** Run an action with busy state and a visible result/error. */
export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const run = useCallback(async <T,>(fn: () => Promise<T>, ok?: (r: T) => string | null): Promise<T | undefined> => {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const r = await fn();
      setMessage(ok ? ok(r) : null);
      return r;
    } catch (e) {
      setError(errorText(e));
      return undefined;
    } finally {
      setBusy(false);
    }
  }, []);
  return { busy, error, message, run, setError, setMessage };
}

export function Notice({ error, message }: { error?: string | null; message?: string | null }) {
  if (error) return <div className="notice error">{error}</div>;
  if (message) return <div className="notice ok">{message}</div>;
  return null;
}

export function Empty({ children }: { children: React.ReactNode }) {
  return <div className="empty">{children}</div>;
}

export function Badge({ tone = "neutral", children }: { tone?: "neutral" | "ok" | "warn" | "err" | "info"; children: React.ReactNode }) {
  return <span className={`badge ${tone}`}>{children}</span>;
}

export function Toggle({ checked, onChange, label, hint, disabled }: { checked: boolean; onChange: (v: boolean) => void; label: string; hint?: string; disabled?: boolean }) {
  return (
    <label className="toggle">
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span>
        <strong>{label}</strong>
        {hint && <small>{hint}</small>}
      </span>
    </label>
  );
}

export function FileSide({ title, view }: { title: string; view: FileView }) {
  return (
    <div className="file-side">
      <div className="file-side-title">{title}</div>
      {!view.exists ? (
        <div className="muted pad">arquivo inexistente nesta versão</div>
      ) : view.binary || view.truncated ? (
        <div className="muted pad">conteúdo binário ou grande demais para exibir ({view.size} bytes)</div>
      ) : (
        <pre className="code">{view.text}</pre>
      )}
    </div>
  );
}

export function DiffView({ left, right, leftTitle, rightTitle }: { left: FileView; right: FileView; leftTitle: string; rightTitle: string }) {
  const textual = (v: FileView) => !v.exists || (v.text !== null && !v.binary);
  if (!textual(left) || !textual(right)) {
    return (
      <div className="split">
        <FileSide title={leftTitle} view={left} />
        <FileSide title={rightTitle} view={right} />
      </div>
    );
  }
  const rows = sideBySide(left.text ?? "", right.text ?? "");
  return (
    <div className="diff">
      <div className="diff-head">
        <div>{leftTitle}</div>
        <div>{rightTitle}</div>
      </div>
      <div className="diff-body">
        {rows.map((r, i) => (
          <div key={i} className={`diff-row ${r.kind}`}>
            <span className="ln">{r.leftNo ?? ""}</span>
            <span className="cell left">{r.left ?? ""}</span>
            <span className="ln">{r.rightNo ?? ""}</span>
            <span className="cell right">{r.right ?? ""}</span>
          </div>
        ))}
        {rows.length === 0 && <div className="muted pad">arquivos vazios</div>}
      </div>
    </div>
  );
}
