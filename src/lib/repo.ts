/** Single source for the project's GitHub URLs — they were drifting across four files. */
export const GITHUB_REPO_URL = "https://github.com/jamesju9999/AITERM";
export const GITHUB_RELEASES_URL = `${GITHUB_REPO_URL}/releases/latest`;

/** The release page for one specific version. `version` carries no "v" prefix. */
export function releaseTagUrl(version: string): string {
  return `${GITHUB_REPO_URL}/releases/tag/v${version}`;
}
