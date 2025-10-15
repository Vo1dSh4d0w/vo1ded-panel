import app from "ags/gtk4/app";
import { Astal, Gtk, Gdk } from "ags/gtk4";
import Time from "./time";
import Tray from "./tray";
import HyprlandWorkspaces from "./hyprland-workspaces";

export default function Bar(gdkmonitor: Gdk.Monitor) {
  const { TOP, LEFT, RIGHT } = Astal.WindowAnchor;

  return (
    <window
      visible
      name="vo1ded-panel"
      class="bar"
      gdkmonitor={gdkmonitor}
      exclusivity={Astal.Exclusivity.EXCLUSIVE}
      anchor={TOP | LEFT | RIGHT}
      application={app}
    >
      <centerbox cssName="centerbox">
        <box $type="center" hexpand halign={Gtk.Align.CENTER}>
          <HyprlandWorkspaces></HyprlandWorkspaces>
        </box>
        <box $type="end" hexpand halign={Gtk.Align.END} spacing={8}>
          <Tray></Tray>
          <Time></Time>
        </box>
      </centerbox>
    </window>
  );
}
