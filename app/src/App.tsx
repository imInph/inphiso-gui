import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { backend, isMock } from "./backend";
import { DriveGrid } from "./components/DriveGrid";
import { Header } from "./components/Header";
import { ArrowRight, Sliders } from "./components/icons";
import { ImageArea } from "./components/ImageArea";
import {
  ChecksumModal,
  ConfirmModal,
  ImageTypeModal,
  LinkModal,
  SettingsModal,
} from "./components/Modals";
import { Cancelled, Done, Failed } from "./components/Outcome";
import { Writing } from "./components/Writing";
import { shortImageName } from "./format";
import { driveFits, flashBlocker, initialState, reducer } from "./state";
import type { FlashEvent, FlashJob } from "./types";
import { useSettings } from "./useSettings";

const DRIVE_POLL_MS = 2000;
const VERSION = "0.1.0";

const errorMessage = (e: unknown) => (e instanceof Error ? e.message : String(e));

export default function App() {
  const [state, dispatch] = useReducer(reducer, initialState);
  const [settings, updateSettings] = useSettings();
  const [ejected, setEjected] = useState(false);
  const [ejecting, setEjecting] = useState(false);
  // Bumped whenever the image changes so stale async results are dropped.
  const imageToken = useRef(0);

  const image = state.image.kind === "ready" ? state.image.info : null;
  const pickedDrive = state.drives.find((d) => d.id === state.pickedDriveId);

  // Drive list, polled for hotplug.
  useEffect(() => {
    let alive = true;
    const refresh = () =>
      backend
        .listDrives(settings.showAllDrives)
        .then((drives) => alive && dispatch({ type: "drives", drives }))
        .catch(() => {});
    refresh();
    const t = setInterval(refresh, DRIVE_POLL_MS);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [settings.showAllDrives]);

  const loadImage = useCallback(async (path: string) => {
    const token = ++imageToken.current;
    dispatch({ type: "imageLoading", name: path.split(/[\\/]/).pop() ?? path });
    try {
      const info = await backend.inspectImage(path);
      if (token === imageToken.current) dispatch({ type: "imageReady", info });
    } catch (e) {
      if (token === imageToken.current) dispatch({ type: "imageError", message: errorMessage(e) });
    }
  }, []);

  // Drag and drop onto the window, only while picking.
  const screenRef = useRef(state.screen);
  screenRef.current = state.screen;
  useEffect(
    () =>
      backend.onFileDrop((path) => {
        if (screenRef.current === "select") loadImage(path);
      }),
    [loadImage],
  );

  // Hash the image whenever it or the pasted checksum changes.
  const imagePath = image?.path ?? null;
  useEffect(() => {
    if (!imagePath) return;
    const token = imageToken.current;
    backend
      .checksum(imagePath, state.expected, (s) => {
        if (token === imageToken.current) dispatch({ type: "checksum", state: s });
      })
      .then((s) => token === imageToken.current && dispatch({ type: "checksum", state: s }))
      .catch(() => token === imageToken.current && dispatch({ type: "checksum", state: { status: "idle" } }));
  }, [imagePath, state.expected]);

  const browse = async () => {
    const path = await backend.pickImage();
    if (path) loadImage(path);
  };

  const download = async (url: string) => {
    const token = ++imageToken.current;
    dispatch({ type: "downloadStart", url });
    try {
      const path = await backend.download(url, settings.downloadDir, (progress) => {
        if (token === imageToken.current) dispatch({ type: "downloadProgress", progress });
      });
      if (token === imageToken.current) loadImage(path);
    } catch (e) {
      if (token === imageToken.current) dispatch({ type: "imageError", message: errorMessage(e) });
    }
  };

  const cancelDownload = () => {
    imageToken.current++;
    backend.cancelDownload();
    dispatch({ type: "clearImage" });
  };

  const replaceImage = () => {
    imageToken.current++;
    dispatch({ type: "clearImage" });
  };

  const eject = async (driveId: string) => {
    setEjecting(true);
    try {
      await backend.eject(driveId);
      setEjected(true);
    } catch {
      // Leave the button enabled so the user can retry.
    } finally {
      setEjecting(false);
    }
  };

  const startFlash = () => {
    if (!image || !pickedDrive || !state.mode) return;
    const job: FlashJob = {
      imagePath: image.path,
      driveId: pickedDrive.id,
      driveSize: pickedDrive.size,
      mode: state.mode,
      verify: settings.verify,
      partitionScheme: settings.partitionScheme,
    };
    setEjected(false);
    dispatch({ type: "flashStart" });
    const onEvent = (event: FlashEvent) => {
      dispatch({ type: "flashEvent", event });
      if (event.type === "done" && settings.ejectWhenDone) eject(job.driveId);
    };
    backend.flash(job, onEvent).catch((e) => dispatch({ type: "flashFailed", message: errorMessage(e) }));
  };

  const blocker = flashBlocker(state);
  const removableCount = state.drives.filter((d) => d.removable).length;

  return (
    <div className="app">
      {state.screen === "select" && (
        <>
          <Header
            right={
              <button
                className="icon-btn"
                aria-label="Settings"
                onClick={() => dispatch({ type: "modal", modal: "settings" })}
              >
                <Sliders />
              </button>
            }
          />
          <main className="body">
            <div className="label">Image</div>
            <ImageArea
              image={state.image}
              checksum={state.checksum}
              checksumOverride={state.checksumOverride}
              mode={state.mode}
              onBrowse={browse}
              onPasteLink={() => dispatch({ type: "modal", modal: "link" })}
              onReplace={replaceImage}
              onCancelDownload={cancelDownload}
              onPasteChecksum={() => dispatch({ type: "modal", modal: "checksum" })}
              onOverrideChecksum={() => dispatch({ type: "overrideChecksum" })}
              onChangeMode={() => dispatch({ type: "modal", modal: "imageType" })}
            />
            <div className="label-row">
              <div className="label">Drive</div>
              <div className="label-meta mono">
                {settings.showAllDrives
                  ? `${state.drives.length} drive${state.drives.length === 1 ? "" : "s"} · system disk hidden`
                  : `${removableCount} removable${image ? " · system disks hidden" : ""}`}
              </div>
            </div>
            <DriveGrid
              drives={state.drives}
              loaded={state.drivesLoaded}
              pickedId={state.pickedDriveId}
              inactive={!image}
              fits={(d) => driveFits(d, state.image)}
              onPick={(id) => dispatch({ type: "pickDrive", id })}
            />
          </main>
          <footer className="footer">
            {image ? (
              <div className="footer-left">
                <label className="check">
                  <input
                    type="checkbox"
                    checked={settings.verify}
                    onChange={(e) => updateSettings({ verify: e.target.checked })}
                  />
                  Verify after writing
                </label>
                <div className="footer-note">
                  {blocker ??
                    (pickedDrive && (
                      <>
                        Erases everything on <b>{pickedDrive.name}</b>
                      </>
                    ))}
                </div>
              </div>
            ) : (
              <div className="footer-note">{blocker}</div>
            )}
            <button
              className="btn-primary flash"
              disabled={!!blocker}
              onClick={() => dispatch({ type: "modal", modal: "confirm" })}
            >
              Flash
              {!blocker && <ArrowRight />}
            </button>
          </footer>
        </>
      )}

      {state.screen === "writing" && (
        <>
          <Header
            right={
              <div className="header-meta mono">
                {image && pickedDrive ? `${shortImageName(image.name)} → ${pickedDrive.name}` : ""}
              </div>
            }
          />
          <main className="body">
            <Writing progress={state.progress} speeds={state.speeds} verify={settings.verify} />
          </main>
          <footer className="footer">
            <div className="footer-note mono">{pickedDrive?.id}</div>
            <button className="btn-outline lg" onClick={() => backend.cancelFlash()}>
              Cancel
            </button>
          </footer>
        </>
      )}

      {state.screen === "done" && image && state.result && (
        <>
          <Header />
          <Done
            image={image}
            drive={pickedDrive}
            checksum={state.checksum}
            elapsedMs={state.result.elapsedMs}
            verified={state.result.verified}
            ejected={ejected}
            ejecting={ejecting}
            onAnother={() => {
              imageToken.current++;
              dispatch({ type: "flashAnother" });
            }}
            onEject={() => state.pickedDriveId && eject(state.pickedDriveId)}
          />
        </>
      )}

      {state.screen === "failed" && (
        <>
          <Header />
          <Failed
            message={state.error ?? "Something went wrong."}
            onBack={() => dispatch({ type: "backToSelect" })}
            onRetry={() => dispatch({ type: "modal", modal: "confirm" })}
          />
        </>
      )}

      {state.screen === "cancelled" && (
        <>
          <Header />
          <Cancelled onBack={() => dispatch({ type: "backToSelect" })} />
        </>
      )}

      {state.modal === "confirm" && image && pickedDrive && state.mode && (
        <ConfirmModal
          drive={pickedDrive}
          image={image}
          mode={state.mode}
          scheme={settings.partitionScheme}
          onCancel={() => dispatch({ type: "modal", modal: null })}
          onConfirm={startFlash}
        />
      )}
      {state.modal === "imageType" && (
        <ImageTypeModal
          current={state.mode}
          onPick={(mode) => dispatch({ type: "setMode", mode })}
          onClose={() => dispatch({ type: "modal", modal: null })}
        />
      )}
      {state.modal === "link" && (
        <LinkModal onSubmit={download} onClose={() => dispatch({ type: "modal", modal: null })} />
      )}
      {state.modal === "checksum" && (
        <ChecksumModal
          initial={state.expected}
          onSubmit={(hash) => dispatch({ type: "setExpected", hash })}
          onClose={() => dispatch({ type: "modal", modal: null })}
        />
      )}
      {state.modal === "settings" && (
        <SettingsModal
          settings={settings}
          version={VERSION}
          onChange={updateSettings}
          onPickDownloadDir={async () => {
            const dir = await backend.pickDirectory();
            if (dir) updateSettings({ downloadDir: dir });
          }}
          onClose={() => dispatch({ type: "modal", modal: null })}
        />
      )}

      {isMock && <div className="mock-badge">mock backend</div>}
    </div>
  );
}
