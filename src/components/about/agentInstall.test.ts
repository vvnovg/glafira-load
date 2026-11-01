import { describe, expect, it } from "vitest";
import { RELEASES_DOWNLOAD_URL } from "../../lib/links";
import { agentInstallCommand } from "./agentInstall";

describe("agentInstallCommand", () => {
  it("macOS и Linux: sudo curl -f в /usr/local/bin и chmod", () => {
    const cmd = agentInstallCommand({ os: "macos", arch: "aarch64", version: "0.2.0" });
    expect(cmd).toBe(
      `sudo curl -fL ${RELEASES_DOWNLOAD_URL}/v0.2.0/glafira-load-macos-aarch64 -o /usr/local/bin/glafira-load && sudo chmod +x /usr/local/bin/glafira-load`,
    );
    expect(agentInstallCommand({ os: "linux", arch: "x86_64", version: "0.2.0" })).toContain("/v0.2.0/glafira-load-linux-x86_64 -o");
  });

  it("Windows: curl.exe -f, .exe и создание папки", () => {
    expect(agentInstallCommand({ os: "windows", arch: "x86_64", version: "0.2.0" })).toBe(
      `curl.exe -fL ${RELEASES_DOWNLOAD_URL}/v0.2.0/glafira-load-windows-x86_64.exe --create-dirs -o C:\\glafira-load\\glafira-load.exe`,
    );
  });

  it("неизвестные os/arch подставляются как есть — команду всё равно видно", () => {
    expect(agentInstallCommand({ os: "freebsd", arch: "riscv64", version: "1.0.0" })).toContain("glafira-load-freebsd-riscv64");
  });
});
