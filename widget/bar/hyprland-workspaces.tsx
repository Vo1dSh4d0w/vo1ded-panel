import { Gdk, Gtk } from "ags/gtk4";
import AstalApps from "gi://AstalApps?version=0.1";
import AstalHyprland from "gi://AstalHyprland?version=0.1";
import GLib from "gi://GLib?version=2.0";
import { createBinding, For, With } from "gnim";

const PID_MIME_TYPE = "text/x-vo1ded-pid";

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
          <button
            class={createBinding(hyprland, "focusedWorkspace").as((ws) =>
              ws.id == workspace.id ? "workspace-active" : "workspace-inactive",
            )}
            onClicked={() => workspace.focus()}
            $={(self) => {
              const dropTarget = new Gtk.DropTargetAsync({
                actions: Gdk.DragAction.COPY,
                formats: new Gdk.ContentFormats([PID_MIME_TYPE]),
              });

              dropTarget.connect("accept", (_, drop) =>
                drop.get_formats().contain_mime_type(PID_MIME_TYPE),
              );

              dropTarget.connect("drag-motion", () => Gdk.DragAction.COPY);

              dropTarget.connect("drop", (_, drop) => {
                drop.read_async(
                  [PID_MIME_TYPE],
                  GLib.PRIORITY_DEFAULT,
                  null,
                  (_, res) => {
                    const [stream] = drop.read_finish(res);
                    const bytes = stream!.read_bytes(4, null);
                    const arr = bytes.get_data();
                    if (arr != null) {
                      let pid = 0;
                      for (let i = 0; i < 4; i++) {
                        pid += arr[i] << (8 * i);
                      }
                      hyprland.dispatch(
                        "movetoworkspacesilent",
                        `${workspace.id}, pid:${pid}`,
                      );
                    }

                    stream?.close(null);
                  },
                );
                drop.finish(Gdk.DragAction.COPY);
              });
              self.add_controller(dropTarget);
            }}
          >
            <box>
              <label
                label={createBinding(workspace, "id").as((id) => id.toString())}
              ></label>
              <For each={createBinding(workspace, "clients")}>
                {(client: AstalHyprland.Client) => {
                  let icon = (
                    <image iconName={client.class}></image>
                  ) as Gtk.Image;
                  for (const app of apps.list) {
                    if (
                      client.class &&
                      app.entry
                        .split(".desktop")[0]
                        .toLowerCase()
                        .match(client.class.toLowerCase())
                    ) {
                      icon = (
                        <image iconName={app.iconName}></image>
                      ) as Gtk.Image;
                    }
                  }

                  const dragSource = new Gtk.DragSource();

                  dragSource.set_actions(Gdk.DragAction.COPY);

                  dragSource.connect("prepare", () => {
                    const pid = client.pid;
                    const arr = new Uint8Array(4);
                    for (let i = 0; i < 4; i++) {
                      arr[i] = pid >> (i * 8);
                    }

                    return Gdk.ContentProvider.new_union([
                      Gdk.ContentProvider.new_for_bytes(
                        PID_MIME_TYPE,
                        new Uint8Array(arr),
                      ),
                      Gdk.ContentProvider.new_for_bytes(
                        "application/octet-stream",
                        new Uint8Array(arr),
                      ),
                      Gdk.ContentProvider.new_for_bytes(
                        "text/plain",
                        new TextEncoder().encode(pid.toString()),
                      ),
                    ]);
                  });

                  dragSource.connect("drag-begin", (_, drag) => {
                    const dragIcon = Gtk.DragIcon.get_for_drag(drag);
                    dragIcon.set_child(
                      (<image iconName={icon.iconName}></image>) as Gtk.Image,
                    );
                    icon.set_visible(false);
                  });

                  dragSource.connect("drag-end", (_, drag) => {
                    icon.set_visible(true);
                  });

                  dragSource.connect("drag-cancel", () => {
                    icon.set_visible(true);
                  });

                  icon.add_controller(dragSource);

                  return icon;
                }}
              </For>
            </box>
          </button>
        )}
      </For>
    </box>
  );
}
