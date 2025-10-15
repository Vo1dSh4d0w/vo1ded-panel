import AstalApps from "gi://AstalApps?version=0.1";
import AstalHyprland from "gi://AstalHyprland?version=0.1";
import { createBinding, For } from "gnim";

export default function HyprlandWorkspaces() {
  const hyprland = AstalHyprland.get_default();
  const apps = new AstalApps.Apps({
    nameMultiplier: 2,
    entryMultiplier: 0,
    executableMultiplier: 2,
  });
  const workspaces = createBinding(hyprland, "workspaces").as((wss) =>
    wss.sort((a, b) => a.id - b.id),
  );

  return (
    <box>
      <For each={workspaces}>
        {(workspace) => (
          <button>
            <box>
              <label label={workspace.id.toString()}></label>
              <For each={createBinding(workspace, "clients")}>
                {(client: AstalHyprland.Client) => {
                  for (const app of apps.list) {
                    if (
                      client.class &&
                      app.entry
                        .split(".desktop")[0]
                        .toLowerCase()
                        .match(client.class.toLowerCase())
                    ) {
                      return <image iconName={app.iconName}></image>;
                    }
                  }
                  return <image iconName={client.class}></image>;
                }}
              </For>
            </box>
          </button>
        )}
      </For>
    </box>
  );
}
