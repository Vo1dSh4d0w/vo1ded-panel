import Astal from "gi://Astal?version=4.0";
import AstalWp from "gi://AstalWp?version=0.1";
import Gtk from "gi://Gtk?version=4.0";
import { createBinding, createComputed, createState, For, With } from "gnim";

const wireplumber = AstalWp.get_default();
const audio = wireplumber.get_audio();

const defaultSpeakerId = createBinding(wireplumber.default_speaker, "id");
const defaultMicrophoneId = createBinding(wireplumber.default_microphone, "id");

const [menuExpanded, setMenuExpanded] = createState(false);

type WireplumberEndpointProps = {
  endpoint: AstalWp.Endpoint;
};

function WireplumberEndpoint({ endpoint }: WireplumberEndpointProps) {
  const slider = (
    <slider
      class={"horizontal marks-before wireplumber-endpoint-slider"}
      value={createBinding(endpoint, "volume")}
      hexpand={true}
      widthRequest={260}
      min={0}
      max={1.5}
      onChangeValue={(_, __, v) => {
        endpoint.volume = v;
        if (v <= 0 && !endpoint.mute) {
          endpoint.mute = true;
        } else if (v > 0 && endpoint.mute) {
          endpoint.mute = false;
        }
        return true;
      }}
    ></slider>
  ) as Astal.Slider;

  slider.add_mark(1.0, Gtk.PositionType.LEFT, "");

  return (
    <box
      visible={createComputed(
        [
          defaultSpeakerId,
          defaultMicrophoneId,
          createBinding(endpoint, "id"),
          menuExpanded,
        ],
        (defaultSpeakerId, defaultMicrophoneId, speakerId, menuExpanded) =>
          menuExpanded ||
          defaultSpeakerId === speakerId ||
          defaultMicrophoneId === speakerId,
      )}
      orientation={Gtk.Orientation.VERTICAL}
    >
      <label
        label={createBinding(endpoint, "description")}
        maxWidthChars={1}
        vexpand={true}
        wrap={true}
      ></label>
      <box>
        <button
          class={"btn-transparent"}
          onClicked={() => {
            endpoint.mute = !endpoint.mute;
          }}
        >
          <image
            iconName={createBinding(endpoint, "volumeIcon").as((iconName) =>
              iconName === "audio-volume-overamplified-symbolic"
                ? "audio-volume-high-symbolic"
                : iconName,
            )}
          ></image>
        </button>
        <box hexpand={true}>{slider}</box>
        <label
          widthRequest={40}
          label={createBinding(endpoint, "volume").as(
            (v) => (v * 100).toFixed(0) + "%",
          )}
        ></label>
      </box>
    </box>
  );
}

function WireplumberPopover() {
  return (
    <box orientation={Gtk.Orientation.VERTICAL}>
      <Astal.Bin>
        <With value={createBinding(audio, "speakers")}>
          {(speakers: AstalWp.Endpoint[]) => (
            <box orientation={Gtk.Orientation.VERTICAL}>
              <For
                each={defaultSpeakerId.as((id) =>
                  speakers.toSorted((speaker) => (speaker.id === id ? -1 : 1)),
                )}
              >
                {(speaker: AstalWp.Endpoint) => (
                  <WireplumberEndpoint endpoint={speaker}></WireplumberEndpoint>
                )}
              </For>
            </box>
          )}
        </With>
      </Astal.Bin>

      <Gtk.Separator></Gtk.Separator>

      <Astal.Bin>
        <With value={createBinding(audio, "microphones")}>
          {(microphones: AstalWp.Endpoint[]) => (
            <box orientation={Gtk.Orientation.VERTICAL}>
              <For
                each={defaultMicrophoneId.as((id) =>
                  microphones.toSorted((microphone) =>
                    microphone.id === id ? -1 : 1,
                  ),
                )}
              >
                {(microphone: AstalWp.Endpoint) => (
                  <WireplumberEndpoint
                    endpoint={microphone}
                  ></WireplumberEndpoint>
                )}
              </For>
            </box>
          )}
        </With>
      </Astal.Bin>

      <button
        label={menuExpanded.as((menuExpanded) =>
          menuExpanded ? "Collapse" : "Expand",
        )}
        onClicked={() => setMenuExpanded(!menuExpanded.get())}
      ></button>
    </box>
  );
}

export default function Wireplumber() {
  return (
    <box>
      <menubutton>
        <image
          iconName={createBinding(wireplumber.default_speaker, "volumeIcon").as(
            (iconName) =>
              iconName === "audio-volume-overamplified-symbolic"
                ? "audio-volume-high-symbolic"
                : iconName,
          )}
        ></image>
        <popover>
          <WireplumberPopover></WireplumberPopover>
        </popover>
      </menubutton>
    </box>
  );
}
