# lu-battery — Konzept

Kleines, immer sichtbares Overlay, das den Akkustand von Razer-Geräten anzeigt
(Maus, Headset, …). Rust, Windows zuerst. Optik: lu-minal-Theme **Moon**.

## Ziel-Geräte (beim User vorhanden)

| Gerät | VID:PID | Akku | Protokoll |
|---|---|---|---|
| Razer Basilisk V3 Pro (Dongle) | 1532:00AB | ja | Razer-HID-Protokoll, 90-Byte Feature Report, Class 0x07 / Cmd 0x80 (Level), 0x84 (Laden), Transaction-ID 0x1F |
| Razer Barracuda X 2.4 (Dongle) | 1532:0550 | ja | Herstellerspezifisch, Interface MI_03 (Vendor-Collection). **Muss ermittelt werden** (nur lesende Abfragen!) |
| Razer Huntsman V3 Pro Mini | 1532:02B0 | nein (Kabel) | wird nicht angezeigt |

Weitere Razer-Mäuse mit dem Standard-Protokoll sollen über eine Tabelle
(PID → Name, Kind, Transaction-ID) leicht ergänzbar sein.

## Anforderungen

1. **Genau**: Angezeigt wird der echte Rohwert vom Gerät (`raw / raw_max`),
   ohne Schätzung oder Glättung. Optional eine Nachkommastelle.
   Lade-Status kommt vom Gerät. Schlägt ein Lesen fehl, bleibt der letzte gute
   Wert stehen und wird als veraltet markiert, nicht als 0 %.
   Gerät schläft/aus → „schläft“ statt falscher Zahl.
2. **Überall**: randloses, transparentes Fenster, immer im Vordergrund, nicht in
   der Taskleiste. Wahlweise in einer der 4 Ecken (mit Abstand) oder frei per Drag;
   Monitor wählbar. Läuft über Spielen im randlosen Fenstermodus.
3. **Einstellbar**: Einstellungsfenster (Rechtsklick aufs Overlay oder Tray-Menü):
   Position/Ecke, Abstand, Monitor, Größe (Scale), Hintergrund-Transparenz,
   horizontal/vertikal, Prozent/Balken/Namen ein/aus, präzise Prozent,
   Klick-Durchlässigkeit, Abfrage-Intervall, Warn- und Kritisch-Schwelle, Geräte ausblenden,
   Autostart. Sofort live sichtbar, gespeichert in `%APPDATA%\lu-battery\settings.json`.
4. **Tray-Icon** mit Menü: Einstellungen, Jetzt aktualisieren, Overlay ein/aus, Beenden.
5. Leicht: wenig CPU (repaint nur bei neuen Daten/Interaktion), eine einzelne `.exe`.

## Architektur

```
main.rs        Start, Logging, eframe-Fenster (transparent, ohne Rahmen, immer oben)
app.rs         eframe::App: Zustand, Fensterposition aus Anchor, Settings-Viewport,
               Tray-Events, wendet UiAction an
settings.rs    Laden/Speichern von model::Settings (JSON), Autostart (HKCU\…\Run)
tray.rs        Tray-Icon + Menü
razer/mod.rs   Hintergrund-Thread: findet Geräte (hidapi), pollt alle poll_secs,
               schickt Vec<DeviceStatus> über einen Channel, ctx.request_repaint()
razer/protocol.rs   Razer-90-Byte-Report bauen/prüfen (CRC), senden/empfangen
razer/devices.rs    Geräte-Tabelle (PID → Name, Kind, Treiber)
razer/barracuda.rs  Headset-Protokoll
model.rs       gemeinsamer Vertrag (DeviceStatus, Settings, UiAction): gehört luna
ui/mod.rs      apply_theme(), overlay(), settings_panel()
ui/theme.rs    Moon-Palette → egui::Visuals
ui/widgets.rs  Akku-Pille, Geräte-Icons (gezeichnet, keine Fremd-Assets)
assets/        App-Icon (.ico/.png), eigenes Design
```

### Schnittstelle UI ↔ App (fest)

```rust
// ui/mod.rs
pub fn apply_theme(ctx: &egui::Context, settings: &Settings);
/// Zeichnet das Overlay in den zentralen Bereich des Hauptfensters.
/// Liefert die gewünschte Fenstergröße (logical px) für den Inhalt zurück.
pub fn overlay(ui: &mut egui::Ui, devices: &[DeviceStatus], settings: &Settings,
               actions: &mut Vec<UiAction>) -> egui::Vec2;
/// Inhalt des Einstellungsfensters (die App öffnet den Viewport).
pub fn settings_panel(ui: &mut egui::Ui, settings: &mut Settings,
                      devices: &[DeviceStatus], actions: &mut Vec<UiAction>);
```

Die App passt die Fenstergröße an den zurückgegebenen Wert an und rechnet die
Position aus Anchor + Margin + Monitor-Arbeitsbereich.

## Theme „Moon“ (aus lu-minal)

| Rolle | Farbe |
|---|---|
| bg | `#0b0920` |
| surface | `#141030` |
| surfaceFocus | `#1d1742` |
| popover | `#181338` |
| border | surface gemischt mit `#b9aefb` 18 % |
| text | `#fbf7f0` |
| muted | surface gemischt mit text 62 % |
| faint | surface gemischt mit text 40 % |
| accent | `#d6cffd` (lavender300) |
| accent2 | `#9ad7f5` (sky300) |
| success / Akku ok | `#7fe3c6` (mint400) |
| warning / Akku niedrig | `#f3c766` |
| error / kritisch | `#e5626b` |
| selection | `#3b2e6b` (violet700) |
| weitere | lavender400 `#b9aefb`, lavender600 `#8f82e0`, outline `#5b4f9e`, moon100 `#f4f1ff`, peach300 `#f7b89a`, pink `#f6a8c8` |

## Team / Zuständigkeiten

| Wer | Dateien |
|---|---|
| luna (Lead, Konzept, Merge, Release) | `CONCEPT.md`, `README.md`, `src/model.rs` |
| design | `src/ui/**`, `assets/**`, `docs/DESIGN.md` |
| code | `Cargo.toml`, `build.rs`, `src/main.rs`, `src/app.rs`, `src/settings.rs`, `src/tray.rs`, `src/razer/**`, `.github/workflows/**` |

Jeder arbeitet auf einem eigenen Branch (`design`, `code`); luna merged in `main`
und macht das Release `v0.1.0` (Windows x64 `.exe`, gebaut von GitHub Actions).
