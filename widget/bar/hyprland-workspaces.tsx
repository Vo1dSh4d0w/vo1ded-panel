import { Gdk, Gtk } from "ags/gtk4";
import AstalApps from "gi://AstalApps?version=0.1";
import AstalHyprland from "gi://AstalHyprland?version=0.1";
import GLib from "gi://GLib?version=2.0";
import {
  Accessor,
  createBinding,
  createComputed,
  createState,
  For,
  With,
} from "ags";
import tryGrabSteamIcon from "../../utils/steam-icon-grabber";
import Astal from "gi://Astal?version=4.0";

const hyprland = AstalHyprland.get_default();
const apps = new AstalApps.Apps({
  nameMultiplier: 2,
  entryMultiplier: 0,
  executableMultiplier: 2,
});

type WorkspaceProps = {
  workspaces: Accessor<AstalHyprland.Workspace[]>;
  focused: Accessor<AstalHyprland.Workspace>;
  specialWorkspace: Accessor<AstalHyprland.Workspace>;
};

type WorkspaceClientsProps = {
  workspace: AstalHyprland.Workspace;
};

function WorkspaceClients({ workspace }: WorkspaceClientsProps) {
  return (
    <For each={createBinding(workspace, "clients")}>
      {(client: AstalHyprland.Client) => {
        let icon: Gtk.Image | undefined;
        for (const app of apps.list) {
          if (
            client.class &&
            app.entry
              .split(".desktop")[0]
              .toLowerCase()
              .match(client.class.toLowerCase())
          ) {
            icon = (<image iconName={app.iconName}></image>) as Gtk.Image;
          }
        }

        if (icon == undefined) {
          const steamIcon = tryGrabSteamIcon(client.pid, client.class);

          if (steamIcon) {
            icon = (<image file={steamIcon}></image>) as Gtk.Image;
          } else {
            icon = (<image iconName={client.class}></image>) as Gtk.Image;
          }
        }

        return icon;
      }}
    </For>
  );
}

function Workspaces({ workspaces, focused, specialWorkspace }: WorkspaceProps) {
  return (
    <box>
      <For each={workspaces}>
        {(workspace) => {
          return (
            <button
              class={createComputed(
                [focused, specialWorkspace],
                (focused, specialWorkspace) =>
                  focused.id === workspace.id ||
                  specialWorkspace?.id === workspace.id
                    ? "workspace workspace-active"
                    : "workspace workspace-inactive",
              )}
              onClicked={() => {
                if (workspace.id < 0) {
                  hyprland.dispatch(
                    "togglespecialworkspace",
                    workspace.name.split(":")[1],
                  );
                } else {
                  workspace.focus();
                }
              }}
            >
              <box>
                <label
                  label={createBinding(workspace, "id").as((id) => {
                    if (id < 0) {
                      return workspace.name.split(":")[1][0].toUpperCase();
                    } else {
                      return workspace.id.toString();
                    }
                  })}
                ></label>
                <WorkspaceClients workspace={workspace}></WorkspaceClients>
              </box>
            </button>
          );
        }}
      </For>
    </box>
  );
}

export default function HyprlandWorkspaces() {
  const workspaces = createBinding(hyprland, "workspaces").as((wss) =>
    wss.sort((a, b) => a.id - b.id),
  );
  const focused = createBinding(hyprland, "focusedWorkspace");
  const monitor = createBinding(hyprland, "focusedMonitor");
  return (
    <box>
      <With value={monitor}>
        {(m) => (
          <Workspaces
            workspaces={workspaces}
            focused={focused}
            specialWorkspace={createBinding(m, "specialWorkspace")}
          ></Workspaces>
        )}
      </With>
    </box>
  );
}
