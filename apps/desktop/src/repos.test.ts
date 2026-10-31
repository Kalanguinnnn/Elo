import { describe, expect, it } from "vitest";
import { cloneFolderName, filterRepos, type Repo } from "./repos";

const repo = (full_name: string, description: string | null = null): Repo => ({
  full_name, private: true, clone_url: `https://github.com/${full_name}.git`, description, updated_at: null,
});
const list = [repo("Kalanguinnnn/Elo", "Continuidade de projetos"), repo("Kalanguinnnn/site"), repo("acme/Ferramentas", "Utilitários de build")];

describe("repository picker", () => {
  it("matches name and description, ignoring case and accents, word by word", () => {
    expect(filterRepos(list, "")).toHaveLength(3);
    expect(filterRepos(list, "elo").map((r) => r.full_name)).toEqual(["Kalanguinnnn/Elo"]);
    expect(filterRepos(list, "utilitarios").map((r) => r.full_name)).toEqual(["acme/Ferramentas"]);
    expect(filterRepos(list, "kalang site").map((r) => r.full_name)).toEqual(["Kalanguinnnn/site"]);
    expect(filterRepos(list, "nada")).toEqual([]);
  });
  it("derives the clone folder like git clone", () => {
    expect(cloneFolderName("https://github.com/a/Elo.git")).toBe("Elo");
    expect(cloneFolderName("git@github.com:a/meu-app.git")).toBe("meu-app");
    expect(cloneFolderName("https://github.com/a/repo/")).toBe("repo");
    expect(cloneFolderName("")).toBe("projeto");
  });
});
