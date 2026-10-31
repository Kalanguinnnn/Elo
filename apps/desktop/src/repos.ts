/** A repository from the connected GitHub account (`github.repos`). */
export interface Repo {
  full_name: string;
  private: boolean;
  clone_url: string;
  description: string | null;
  updated_at: string | null;
}

const fold = (s: string) => s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();

/** Repositories whose name or description contain every word of `query`. */
export function filterRepos(repos: Repo[], query: string): Repo[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (!words.length) return repos;
  return repos.filter((r) => {
    const hay = fold(`${r.full_name} ${r.description ?? ""}`);
    return words.every((w) => hay.includes(w));
  });
}

/** Folder name `git clone` would create for `url`. */
export function cloneFolderName(url: string): string {
  return url.trim().replace(/\/+$/, "").replace(/\.git$/, "").split(/[/:]/).pop() || "projeto";
}
