# lu-battery

Kleines Overlay, das den Akkustand deiner Razer-Geräte (Maus, Headset) anzeigt, im lu-minal-Theme „Moon“.
Rust, Windows.

## Unterstützt

- Razer Basilisk V3 Pro (Dongle und Kabel) und andere Razer-Mäuse mit dem Standard-Protokoll
- Razer Barracuda X 2.4 (Dongle)

Der Akkustand wird direkt vom Gerät gelesen (Rohwert, z. B. 233/255 = 91,4 %). Läuft Razer Synapse 4,
löst jede Akku-Meldung von Synapse sofort ein neues Lesen aus; schlägt das Lesen fehl, wird der Synapse-Wert genutzt.

## Benutzung

`lu-battery.exe` aus dem [Release](https://github.com/Luna-OS/lu-battery/releases) starten.

- Ziehen: Overlay verschieben
- Doppelklick oder Rechtsklick → Einstellungen: Ecke, Abstand, Monitor, Größe, Transparenz, Layout,
  Prozent/Balken/Namen, Nachkommastelle, Klick-durchlässig, Intervall, Warnschwellen, Geräte, Autostart
- Tray-Icon: Einstellungen, Jetzt aktualisieren, Overlay anzeigen, Beenden
- `lu-battery.exe --probe` zeigt gefundene Geräte und Rohwerte in der Konsole

Einstellungen liegen in `%APPDATA%\lu-battery\settings.json`.

## Bauen

```
cargo build --release
```

Siehe [CONCEPT.md](CONCEPT.md) und [docs/DESIGN.md](docs/DESIGN.md).
