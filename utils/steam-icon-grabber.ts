/**
 * WARNING: VOODOO STEAM SHENANIGANS
 *
 * This file exists because Steam does not believe in things like
 * "stable APIs", "simple metadata", or "making sense".
 *
 * What follows is a delicate archaeological dig through:
 *  - window class guesses
 *  - /proc spelunking
 *  - demonic VDF parsing
 *  - icon caches that may or may not exist today
 *
 * If this breaks:
 *  - It is probably Steam’s fault.
 *  - It is definitely not deterministic.
 *  - Debugging it will make you question your life choices.
 *
 * If you are thinking “I’ll just clean this up real quick”:
 *  - No you won’t.
 *  - You will make it worse.
 *  - Steam will change something tomorrow anyway.
 *
 * Touch this code only if:
 *  - You enjoy pain
 *  - You have accepted that the correct fix may be “it works on my machine”
 *
 * You have been warned. Proceed at your own risk.
 */

import GLib from "gi://GLib?version=2.0";

function findSteamIcon(appId: string): string | undefined {
  const home_path = GLib.getenv("HOME");

  const hicolorDir = GLib.Dir.open(
    `${home_path}/.local/share/icons/hicolor`,
    0,
  );
  while (true) {
    const iconChild = hicolorDir.read_name();
    if (!iconChild) break;

    const iconPath = `${home_path}/.local/share/icons/hicolor/${iconChild}/apps/steam_icon_${appId}.png`;
    if (
      GLib.file_test(
        `${home_path}/.local/share/icons/hicolor/${iconChild}/apps/steam_icon_${appId}.png`,
        GLib.FileTest.EXISTS,
      )
    ) {
      return iconPath;
    }
  }
}

export default function tryGrabSteamIcon(
  pid: number,
  clientClass: string,
): string | undefined {
  if (clientClass.startsWith("steam_app_")) {
    return findSteamIcon(clientClass.substring(10));
  }

  const exe = GLib.file_read_link(`/proc/${pid}/exe`);

  const home_path = GLib.get_home_dir();
  const libFoldersPath = `${home_path}/.local/share/Steam/config/libraryfolders.vdf`;
  const fileRead = GLib.file_get_contents(libFoldersPath);
  const fileContent = new TextDecoder().decode(fileRead[1]);
  let steamDir: string | undefined = undefined;

  for (const match of fileContent.matchAll(/"path"\s+"([^"]+)"/gm)) {
    const path = match[1];
    if (exe.startsWith(path)) {
      steamDir = path;
      break;
    }
  }

  if (steamDir === undefined) return undefined;

  const installDirSuffix = exe.replace(`${steamDir}/steamapps/common/`, "");
  const installDir = installDirSuffix.substring(
    0,
    installDirSuffix.indexOf("/"),
  );

  const steamappsDir = GLib.Dir.open(`${steamDir}/steamapps`, 0);
  while (true) {
    const child = steamappsDir.read_name();
    if (!child) break;
    if (child.startsWith("appmanifest")) {
      const manifestContentEncoded = GLib.file_get_contents(
        `${steamDir}/steamapps/${child}`,
      );
      const manifestContent = new TextDecoder().decode(
        manifestContentEncoded[1],
      );
      const manifestInstallDir = manifestContent.match(
        /"installdir"\s+"([^"]+)"/,
      )?.[1];
      if (manifestInstallDir == installDir) {
        const appId = manifestContent.match(/"appid"\s+"([^"]+)"/)?.[1];

        if (appId) return findSteamIcon(appId);
      }
    }
  }

  return undefined;
}
