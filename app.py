import sys
import requests
import socket
import subprocess
from PyQt6.QtWidgets import (
    QApplication, QMainWindow, QLabel, QVBoxLayout, QHBoxLayout, QWidget, QFrame
)
from PyQt6.QtCore import Qt, QTimer

def get_public_ip():
    try:
        response = requests.get("https://api.ipify.org?format=text", timeout=5)
        response.raise_for_status()
        return response.text
    except Exception:
        return None

def get_local_ip():
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.settimeout(0)
        s.connect(('8.8.8.8', 80))
        ip = s.getsockname()[0]
        s.close()
        return ip
    except Exception:
        return None

def get_gateway_ip():
    try:
        result = subprocess.run(["ip", "route"], capture_output=True, text=True)
        for line in result.stdout.splitlines():
            if line.startswith("default"):
                parts = line.split()
                gw_ip = parts[2]
                return gw_ip
        return None
    except Exception:
        return None

def ping(host):
    if not host or host in ["Unknown", "Could not get gateway IP:"]:
        return False
    try:
        result = subprocess.run(
            ["ping", "-c", "1", "-W", "1", host],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL
        )
        return result.returncode == 0
    except Exception:
        return False

def get_active_connections():
    """
    Returns a list of tuples: (iface_name, type, icon, state_text, color)
    Ethernet: state_text is 'Plugged in' or 'Plugged out' (LOWER_UP present)
    Wi-Fi: state_text is 'Connected' (UP and has IP) or 'Disconnected'
    """
    connections = []
    try:
        # Get all interfaces and their flags
        result = subprocess.run(["ip", "-o", "link"], capture_output=True, text=True)
        lines = result.stdout.splitlines()
        # Get IP addresses for interfaces
        ip_result = subprocess.run(["ip", "-o", "-f", "inet", "addr", "show"], capture_output=True, text=True)
        ip_lines = ip_result.stdout.splitlines()
        iface_ips = {}
        for ip_line in ip_lines:
            ip_parts = ip_line.split()
            if len(ip_parts) >= 4:
                iface_name = ip_parts[1]
                ip_addr = ip_parts[3].split("/")[0]
                iface_ips.setdefault(iface_name, []).append(ip_addr)

        for line in lines:
            parts = line.split(": ")
            if len(parts) < 3:
                continue
            iface = parts[1].split(":")[0]
            flags = parts[2]
            # Ethernet
            if iface.startswith("en") or iface.startswith("eth") or "enp" in iface or "eno" in iface or "eth" in iface:
                conn_type = "Ethernet"
                icon = "🖧"
                if "LOWER_UP" in flags:
                    state_text = "Plugged in"
                    color = "green"
                else:
                    state_text = "Plugged out"
                    color = "red"
                connections.append((iface, conn_type, icon, state_text, color))
            # Wi-Fi
            elif iface.startswith("wl") or "wlan" in iface:
                conn_type = "Wi-Fi"
                icon = "📶"
                has_ip = iface in iface_ips and len(iface_ips[iface]) > 0
                if "UP" in flags and has_ip:
                    state_text = "Connected"
                    color = "green"
                else:
                    state_text = "Disconnected"
                    color = "red"
                connections.append((iface, conn_type, icon, state_text, color))
        return connections
    except Exception:
        return []

class StatusBox(QFrame):
    def __init__(self, title, ip, status, icon):
        super().__init__()
        # Only the outer box has border
        self.setStyleSheet("""
            StatusBox {
                border: 1px solid #888888;
                border-radius: 10px;
                background: transparent;  /* keep it transparent */
            }
        """)

        layout = QVBoxLayout()

        icon_label = QLabel(icon)
        icon_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        icon_label.setStyleSheet("font-size: 32px; border: none; background: transparent;")

        self.title_label = QLabel(title)
        self.title_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.title_label.setStyleSheet("font-size: 16px; font-weight: bold; border: none; background: transparent;")

        self.ip_label = QLabel(ip)
        self.ip_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.ip_label.setStyleSheet("font-size: 14px; border: none; background: transparent;")

        self.status_label = QLabel("✔️ Connected" if status else "❌ Not Connected")
        self.status_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.status_label.setStyleSheet("font-size: 15px; border: none; background: transparent;")

        layout.addWidget(icon_label)
        layout.addWidget(self.title_label)
        layout.addWidget(self.ip_label)
        layout.addWidget(self.status_label)

        self.setLayout(layout)
        self.setFixedSize(250, 200)



    def update(self, ip, status):
        ip_display = ip if ip else "No IP"
        self.ip_label.setText(f"IP: {ip_display}")
        self.status_label.setText("✔️ Connected" if status else "❌ Not Connected")
        self.status_label.setStyleSheet(
            "font-size: 15px;" + ("color: green;" if status else "color: red;")
        )

class CenteredInfoBox(QFrame):
    def __init__(self, text):
        super().__init__()
        self.setStyleSheet("""
            QFrame {
                border: none;
                background: transparent;
                border-radius: 12px;
            }
        """)
        layout = QVBoxLayout()
        self.info_label = QLabel(text)
        self.info_label.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignVCenter)
        self.info_label.setStyleSheet("font-size: 15px;")
        self.info_label.setTextFormat(Qt.TextFormat.RichText)
        self.info_label.setWordWrap(True)
        self.info_label.setText(text)
        layout.addWidget(self.info_label)
        layout.setContentsMargins(12, 10, 12, 10)
        self.setLayout(layout)
        self.setFixedSize(500, 200)

    def set_text(self, text):
        self.info_label.setTextFormat(Qt.TextFormat.RichText)
        self.info_label.setWordWrap(True)
        self.info_label.setText(text)

class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Connection Dashboard")
        self.setMinimumSize(700, 400)
        self.resize(950, 600)

        self.desktop_box = StatusBox("desktop", "IP: ...", False, "💻")
        self.router_box = StatusBox("Router", "IP: ...", False, "📶")
        self.internet_box = StatusBox("Internet", "IP: ...", False, "🌐")

        connect_icon1 = QLabel("⟶")
        connect_icon1.setAlignment(Qt.AlignmentFlag.AlignCenter)
        connect_icon1.setStyleSheet("font-size: 28px;")
        connect_icon2 = QLabel("⟶")
        connect_icon2.setAlignment(Qt.AlignmentFlag.AlignCenter)
        connect_icon2.setStyleSheet("font-size: 28px;")

        boxes_layout = QHBoxLayout()
        boxes_layout.addStretch()
        boxes_layout.addWidget(self.desktop_box)
        boxes_layout.addWidget(connect_icon1)
        boxes_layout.addWidget(self.router_box)
        boxes_layout.addWidget(connect_icon2)
        boxes_layout.addWidget(self.internet_box)
        boxes_layout.addStretch()

        self.info_box = CenteredInfoBox("Checking network status...")

        info_box_layout = QHBoxLayout()
        info_box_layout.addStretch()
        info_box_layout.addWidget(self.info_box)
        info_box_layout.addStretch()

        main_layout = QVBoxLayout()
        main_layout.addStretch()
        main_layout.addLayout(boxes_layout)
        main_layout.addSpacing(20)
        main_layout.addLayout(info_box_layout)
        main_layout.addStretch()

        container = QWidget()
        container.setLayout(main_layout)
        self.setCentralWidget(container)

        self.timer = QTimer()
        self.timer.timeout.connect(self.update_network_status)
        self.timer.start(5000)

        self.update_network_status()

    def update_network_status(self):
        public_ip = get_public_ip()
        local_ip = get_local_ip()
        gateway_ip = get_gateway_ip()

        desktop_status = True if local_ip else False
        router_status = ping(gateway_ip) if gateway_ip else False
        internet_status = ping("8.8.8.8") if router_status else False

        self.desktop_box.update(local_ip, desktop_status)
        self.router_box.update(gateway_ip, router_status)
        self.internet_box.update(public_ip, internet_status)

        info_lines = []
        if desktop_status and router_status and internet_status:
            info_lines.append("✅ Network is working fine.")
        elif not desktop_status:
            info_lines.append("❌ Problem with your desktop network adapter or configuration.")
        elif not router_status:
            info_lines.append("❌ Problem connecting to your router. Check router and cable/wifi.")
        elif not internet_status:
            info_lines.append("❌ Connected to router, but no internet. The issue may be with your ISP.")
        else:
            info_lines.append("⚠️ Unable to determine network status.")

        active_conns = get_active_connections()
        for iface, conn_type, icon, state_text, color in active_conns:
            if conn_type in ("Ethernet", "Wi-Fi"):
                info_lines.append(
                    f'<span style="font-size:18px;">{icon}</span> {conn_type} ({iface}) - <span style="color:{color};">{state_text}</span>'
                )

        self.info_box.set_text("<br>".join(info_lines))

if __name__ == "__main__":
    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())
