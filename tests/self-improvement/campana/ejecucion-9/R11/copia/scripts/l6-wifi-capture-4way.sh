#!/usr/bin/env bash
# Captura un 4-way WPA2-PSK/CCMP real y lo congela como fixture de `soso-wpa2`.
#
# POR QUÉ: el banco del supplicant se contrasta consigo mismo (una segunda
# implementación de la PRF escrita desde la norma, y un AP simulado que deriva
# la PTK por ese otro camino). Eso demuestra coherencia, no corrección. Con una
# transcripción de hostapd y wpa_supplicant —dos implementaciones ajenas y
# ampliamente desplegadas— la prueba pasa a ser: **nuestro M2 y nuestro M4
# tienen que salir byte a byte iguales a los suyos, MIC incluido**. Si eso se
# cumple, PBKDF2, la PRF, la derivación de la PTK, la codificación de
# `key_info`, las longitudes y el HMAC están bien, y no por acuerdo interno.
#
# QUÉ TOCA EN EL SISTEMA (todo temporal, se deshace al salir):
#   - carga `mac80211_hwsim` con 2 radios virtuales (no toca el WiFi real);
#   - levanta hostapd en una y wpa_supplicant en la otra;
#   - captura con tcpdump sólo tramas EAPOL (ethertype 0x888e);
#   - descarga el módulo al terminar.
# No modifica la configuración de red existente ni el WiFi del host.
#
# USO:  sudo ./scripts/l6-wifi-capture-4way.sh
# El fixture queda en crates/soso-wpa2/tests/fixtures/4way-hostapd.txt
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$root/target/wifi-4way-capture"
fixture_dir="$root/crates/soso-wpa2/tests/fixtures"
fixture="$fixture_dir/4way-hostapd.txt"

SSID="${SOSO_4WAY_SSID:-soso-4way-test}"
PSK="${SOSO_4WAY_PSK:-clave-de-laboratorio}"
CHAN=1

if [[ "$(id -u)" != 0 ]]; then
    echo "Hace falta root: mac80211_hwsim y hostapd necesitan privilegios." >&2
    echo "  sudo $0" >&2
    exit 1
fi

falta=()
for b in hostapd wpa_supplicant tcpdump; do
    command -v "$b" >/dev/null || falta+=("$b")
done
if ((${#falta[@]})); then
    echo "faltan binarios: ${falta[*]}" >&2
    echo "  apt install hostapd wpasupplicant tcpdump" >&2
    echo "(hostapd suele quedar enmascarado al instalarlo; este script lo lanza" >&2
    echo " a mano, así que no hace falta habilitar el servicio.)" >&2
    exit 1
fi

mkdir -p "$work" "$fixture_dir"
rm -f "$work"/*.pcap "$work"/*.log "$work"/*.conf

# --- interfaces hwsim: las que aparecen tras cargar el módulo -------------
antes="$(ip -o link | awk -F': ' '{print $2}' | cut -d@ -f1 | sort)"
modprobe mac80211_hwsim radios=2
sleep 1
despues="$(ip -o link | awk -F': ' '{print $2}' | cut -d@ -f1 | sort)"
mapfile -t nuevas < <(comm -13 <(echo "$antes") <(echo "$despues"))

limpieza() {
    set +e
    [[ -n "${tcpdump_pid:-}" ]] && kill "$tcpdump_pid" 2>/dev/null
    [[ -n "${wpa_pid:-}" ]] && kill "$wpa_pid" 2>/dev/null
    [[ -n "${hostapd_pid:-}" ]] && kill "$hostapd_pid" 2>/dev/null
    sleep 1
    rmmod mac80211_hwsim 2>/dev/null
}
trap limpieza EXIT

if ((${#nuevas[@]} < 2)); then
    echo "mac80211_hwsim no creó dos interfaces (vio: ${nuevas[*]:-ninguna})" >&2
    exit 1
fi
AP_IF="${nuevas[0]}"
STA_IF="${nuevas[1]}"
echo "hwsim: AP=$AP_IF  STA=$STA_IF"

# NetworkManager intentaría gestionarlas y competir con wpa_supplicant.
if command -v nmcli >/dev/null; then
    nmcli device set "$AP_IF" managed no 2>/dev/null || true
    nmcli device set "$STA_IF" managed no 2>/dev/null || true
fi

AP_MAC="$(cat "/sys/class/net/$AP_IF/address")"
STA_MAC="$(cat "/sys/class/net/$STA_IF/address")"

# --- hostapd: WPA2-PSK, CCMP sólo ----------------------------------------
cat > "$work/hostapd.conf" <<EOF
interface=$AP_IF
driver=nl80211
ssid=$SSID
hw_mode=g
channel=$CHAN
wpa=2
wpa_passphrase=$PSK
wpa_key_mgmt=WPA-PSK
rsn_pairwise=CCMP
wpa_pairwise=CCMP
EOF

cat > "$work/wpa_supplicant.conf" <<EOF
ctrl_interface=$work/ctrl
network={
    ssid="$SSID"
    psk="$PSK"
    key_mgmt=WPA-PSK
    proto=RSN
    pairwise=CCMP
    group=CCMP
}
EOF

hostapd "$work/hostapd.conf" > "$work/hostapd.log" 2>&1 &
hostapd_pid=$!
sleep 2
if ! kill -0 "$hostapd_pid" 2>/dev/null; then
    echo "hostapd murió; log:" >&2
    cat "$work/hostapd.log" >&2
    exit 1
fi

# Sólo EAPOL: la captura no debe llevarse tráfico ajeno a la prueba.
tcpdump -i "$STA_IF" -s 0 -U -w "$work/4way.pcap" "ether proto 0x888e" \
    > "$work/tcpdump.log" 2>&1 &
tcpdump_pid=$!
sleep 1

wpa_supplicant -i "$STA_IF" -c "$work/wpa_supplicant.conf" -d \
    > "$work/wpa_supplicant.log" 2>&1 &
wpa_pid=$!

echo "esperando al 4-way…"
for _ in $(seq 1 30); do
    grep -q "WPA: Key negotiation completed" "$work/wpa_supplicant.log" && break
    sleep 1
done
sleep 1
kill "$tcpdump_pid" 2>/dev/null || true
wait "$tcpdump_pid" 2>/dev/null || true
unset tcpdump_pid

if ! grep -q "WPA: Key negotiation completed" "$work/wpa_supplicant.log"; then
    echo "el 4-way no terminó; revisa $work/wpa_supplicant.log" >&2
    exit 1
fi

# --- pcap → fixture de texto ---------------------------------------------
# Sin dependencias nuevas: el formato pcap clásico es una cabecera de 24 B y,
# por paquete, 16 B de cabecera más los datos.
python3 - "$work/4way.pcap" "$fixture" "$SSID" "$PSK" "$STA_MAC" "$AP_MAC" <<'PY'
import struct, sys, datetime

pcap, salida, ssid, psk, sta, ap = sys.argv[1:7]
raw = open(pcap, 'rb').read()
magic, = struct.unpack('<I', raw[:4])
if magic == 0xa1b2c3d4:
    end, nano = '<', False
elif magic == 0xd4c3b2a1:
    end, nano = '>', False
elif magic == 0xa1b23c4d:
    end, nano = '<', True
elif magic == 0x4d3cb2a1:
    end, nano = '>', True
else:
    sys.exit(f'pcap con magic desconocido: {magic:#x}')

pos, marcos = 24, []
while pos + 16 <= len(raw):
    _, _, caplen, _ = struct.unpack(end + 'IIII', raw[pos:pos + 16])
    datos = raw[pos + 16:pos + 16 + caplen]
    pos += 16 + caplen
    if len(datos) >= 14 and datos[12:14] == b'\x88\x8e':
        marcos.append(datos)

if len(marcos) < 4:
    sys.exit(f'la captura sólo tiene {len(marcos)} tramas EAPOL; hacen falta 4')
m1, m2, m3, m4 = marcos[:4]

def sentido(m):
    return 'ap->sta' if m[6:12] == bytes.fromhex(ap.replace(':', '')) else 'sta->ap'

esperado = ['ap->sta', 'sta->ap', 'ap->sta', 'sta->ap']
visto = [sentido(m) for m in (m1, m2, m3, m4)]
if visto != esperado:
    sys.exit(f'las 4 primeras tramas no son M1..M4 (sentidos: {visto})')

with open(salida, 'w') as f:
    f.write('# Transcripción real de un 4-way WPA2-PSK/CCMP entre hostapd y\n')
    f.write('# wpa_supplicant sobre mac80211_hwsim. Generado por\n')
    f.write('# scripts/l6-wifi-capture-4way.sh el ')
    f.write(datetime.date.today().isoformat() + '.\n')
    f.write('#\n')
    f.write('# m1..m4 son tramas Ethernet completas (14 B de cabecera + EAPOL),\n')
    f.write('# en el orden en que viajaron. No regenerar a mano.\n')
    f.write(f'ssid={ssid}\n')
    f.write(f'passphrase={psk}\n')
    f.write(f'sta={sta}\n')
    f.write(f'ap={ap}\n')
    for nombre, m in (('m1', m1), ('m2', m2), ('m3', m3), ('m4', m4)):
        f.write(f'{nombre}={m.hex()}\n')
print(f'fixture escrito: {salida} ({len(m1)}/{len(m2)}/{len(m3)}/{len(m4)} bytes)')
PY

# El fixture es del repositorio, no de root.
if [[ -n "${SUDO_UID:-}" ]]; then
    chown "$SUDO_UID:${SUDO_GID:-$SUDO_UID}" "$fixture"
fi

echo
echo "Listo. Ahora:  cargo test -p soso-wpa2"
echo "El test 'transcripcion_real' pasa de saltarse a comparar nuestro M2 y M4"
echo "byte a byte con los de wpa_supplicant."
