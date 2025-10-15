import { Gdk, Gtk } from "ags/gtk4";
import AstalTray from "gi://AstalTray?version=0.1";
import { createBinding, For } from "gnim";

function TrayItem({ item }: { item: AstalTray.TrayItem }) {
  const button = (
    <menubutton
      $={(self) => self.insert_action_group("dbusmenu", item.actionGroup)}
      menuModel={createBinding(item, "menuModel")}
      class="tray-item"
      tooltipMarkup={item.tooltipMarkup}
    >
      <image gicon={item.gicon}></image>
      {Gtk.PopoverMenu.new_from_model_full(
        item.menuModel,
        Gtk.PopoverMenuFlags.NESTED,
      )}
    </menubutton>
  ) as Gtk.MenuButton;

  const controller = new Gtk.EventControllerLegacy({
    propagationPhase: Gtk.PropagationPhase.CAPTURE,
  });

  controller.connect("event", (self, event) => {
    switch (event.get_event_type()) {
      case Gdk.EventType.BUTTON_PRESS: {
        const mouse_button = (event as Gdk.ButtonEvent).get_button();
        if (
          mouse_button == Gdk.BUTTON_SECONDARY ||
          (item.is_menu && mouse_button == Gdk.BUTTON_PRIMARY)
        ) {
          item.about_to_show();
        }
        break;
      }
      case Gdk.EventType.BUTTON_RELEASE: {
        const buttonEvent = event as Gdk.ButtonEvent;
        if (buttonEvent.get_surface() != button.get_native()?.get_surface()) {
          return false;
        }

        const mouse_button = buttonEvent.get_button();
        if (mouse_button == Gdk.BUTTON_PRIMARY) {
          if (item.is_menu) {
            button.popup();
          } else {
            const [_, x, y] = buttonEvent.get_position();
            item.activate(x, y);
          }
        } else if (mouse_button == Gdk.BUTTON_SECONDARY) {
          button.popup();
        }
      }
    }

    return true;
  });

  button.add_controller(controller);
  item.connect("notify::action-group", () => {
    button.insert_action_group("dbusmenu", item.actionGroup);
  });

  return button;
}

export default function Tray() {
  const tray = AstalTray.get_default();
  const items = createBinding(tray, "items");
  const filtered = items((item) => item.filter((i) => i.menuModel != null));

  return (
    <box class="tray" spacing={0}>
      <For each={filtered}>{(item) => <TrayItem item={item}></TrayItem>}</For>
    </box>
  );
}
