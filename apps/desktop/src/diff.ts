import { diffLines } from "diff";

export interface Row {
  kind: "same" | "add" | "del" | "change";
  left: string | null;
  right: string | null;
  leftNo: number | null;
  rightNo: number | null;
}

function lines(s: string): string[] {
  const v = s.split("\n");
  if (v.length && v[v.length - 1] === "") v.pop();
  return v;
}

/** Side-by-side rows; removed+added runs are paired as changes. */
export function sideBySide(oldText: string, newText: string): Row[] {
  const rows: Row[] = [];
  let l = 1;
  let r = 1;
  const parts = diffLines(oldText, newText);
  for (let i = 0; i < parts.length; i++) {
    const p = parts[i];
    if (!p.added && !p.removed) {
      for (const t of lines(p.value)) rows.push({ kind: "same", left: t, right: t, leftNo: l++, rightNo: r++ });
    } else if (p.removed && parts[i + 1]?.added) {
      const a = lines(p.value);
      const b = lines(parts[i + 1].value);
      for (let k = 0; k < Math.max(a.length, b.length); k++) {
        const left = a[k] ?? null;
        const right = b[k] ?? null;
        rows.push({
          kind: left !== null && right !== null ? "change" : left !== null ? "del" : "add",
          left,
          right,
          leftNo: left !== null ? l++ : null,
          rightNo: right !== null ? r++ : null,
        });
      }
      i++;
    } else if (p.removed) {
      for (const t of lines(p.value)) rows.push({ kind: "del", left: t, right: null, leftNo: l++, rightNo: null });
    } else {
      for (const t of lines(p.value)) rows.push({ kind: "add", left: null, right: t, leftNo: null, rightNo: r++ });
    }
  }
  return rows;
}
