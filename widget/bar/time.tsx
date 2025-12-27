import { Gtk } from "ags/gtk4";
import { createPoll } from "ags/time";

export default function Time() {
  const time = createPoll("", 1000, "date '+%Y-%m-%d %H:%M:%S'");

  return (
    <menubutton class="transparent">
      <label label={time} />
      <popover>
        <Gtk.Calendar />
      </popover>
    </menubutton>
  );
}
