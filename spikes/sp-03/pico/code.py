# SP-03 (#5) Pi Pico HID rig: 1,000 talk-key press/release cycles (NFR-08a, NFR-08b, AT-30).
#
# Board: Raspberry Pi Pico (RP2040) with CircuitPython 9.x and the adafruit_hid library in /lib.
# The Pico enumerates as a USB keyboard and presses Right Alt (= Right Option on macOS, the D66
# default talk key), CYCLES times. It sends nothing else.
#
# Wiring (optional):
#   GP15 to GND: start at once (else the run starts after START_DELAY_S).
#   GP14 to GND: abort; the Pico releases every key and stops.
# The onboard LED is on while the key is down.
#
# Serial console (115200, for example `screen /dev/tty.usbmodem* 115200`) prints one line per
# cycle and a final DONE line. Compare DONE with the probe's `analyze` result.

import random
import time

import board
import digitalio
import usb_hid
from adafruit_hid.keyboard import Keyboard
from adafruit_hid.keycode import Keycode

CYCLES = 1000
START_DELAY_S = 10  # time to put the target app in front
HOLD_MS = (50, 150)  # uniform hold time range
GAP_MS = (100, 200)  # uniform gap range
SEED = 3  # fixed seed: the same hold/gap sequence every run
TALK_KEY = Keycode.RIGHT_ALT

led = digitalio.DigitalInOut(board.LED)
led.direction = digitalio.Direction.OUTPUT


def pin(name):
    p = digitalio.DigitalInOut(name)
    p.direction = digitalio.Direction.INPUT
    p.pull = digitalio.Pull.UP
    return p


start_pin = pin(board.GP15)
abort_pin = pin(board.GP14)


def aborted():
    return not abort_pin.value


kbd = Keyboard(usb_hid.devices)
kbd.release_all()
random.seed(SEED)

print("SP03 PICO ready cycles=%d start_delay_s=%d" % (CYCLES, START_DELAY_S))
t_end = time.monotonic() + START_DELAY_S
while time.monotonic() < t_end and start_pin.value:
    led.value = not led.value
    time.sleep(0.25)
led.value = False

done = 0
t0 = time.monotonic_ns()
for i in range(1, CYCLES + 1):
    if aborted():
        break
    hold = random.randint(*HOLD_MS)
    gap = random.randint(*GAP_MS)
    kbd.press(TALK_KEY)
    led.value = True
    time.sleep(hold / 1000)
    kbd.release(TALK_KEY)
    led.value = False
    done = i
    print("CYCLE %d hold_ms=%d" % (i, hold))
    time.sleep(gap / 1000)

kbd.release_all()
print("DONE cycles=%d of %d elapsed_s=%.1f" % (done, CYCLES, (time.monotonic_ns() - t0) / 1e9))
while True:
    time.sleep(1)
