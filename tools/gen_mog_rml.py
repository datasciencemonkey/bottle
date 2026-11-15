#!/usr/bin/env python3
"""Generate mog-rive/scene.rml: "Mog", the MOG mascot of Bottle.

Art direction U14: a super cute genie that rises out of Genie's lamp. The lamp
is a simplified, original drawing that extends the Databricks Genie One lamp
icon (coral FF5F46 / pale pink FABFBA). The genie has a soft pink-to-coral
body, a navy screen face with glowing eyes, blush cheeks, nub arms and a
floating sparkle. There is no smoke tail and no brick torso (UX-10a, UX-10c),
and no fill uses Databricks Lava FF3621 (UX-10b). Brand approval is required
before any external release (SEC-22a, SEC-22b).

It is driven entirely by a view model so any host
(Tauri/Electron web runtime, Flutter rive package, native) can set it:

  mood   enum   idle | listening | processing | answering | success | error | sleeping
  level  number 0..1 mic level (listening rings glow + scale)
  hover  bool   written by the file on pointer enter/exit
  poke   trigger written by the file on click (host may also fire it)
  demo   bool   when true, clicking cycles idle>listening>processing>answering>success>idle
  attention bool "?" bubble over any mood (needs input, sign-in), UX-03
  recording bool red ring and REC dot over any mood (meeting session), UX-04a

At load: mood=idle, level=0, hover=false, demo=false, attention=false,
recording=false (D64, UX-02b).

Event "poked" is reported to the host on every click.

Run:  uv run python tools/gen_mog_rml.py  (writes ../mog-rive/scene.rml)
"""
import math
import os

OUT = os.path.join(os.path.dirname(__file__), "..", "mog-rive", "scene.rml")

_next = [10]


def nid():
    _next[0] += 1
    return f"0:{_next[0]}"


EASE = '<CubicEaseInterpolator x1="0.42" y1="0" x2="0.58" y2="1"/>'
TAU = 6.2831855

# ---------------------------------------------------------------- palette
# Genie One lamp icon colours (Jira AIBI-1851): coral FF5F46, pale pink FABFBA.
# Databricks Navy 800 1B3139 / 900 0B2026 and Oat F9F7F4 / EEEDE9.
# Lava FF3621 is deliberately not used (UX-10b). Shades marked "derived" are
# darker or lighter mixes of the brand coral and need the same brand review.
CORAL = "FFFF5F46"
CORAL_D = "FFD9452F"    # derived: shaded coral
CORAL_DD = "FFB23826"   # derived: deep shade for the lamp foot and handle
PINK = "FFFABFBA"
NAVY = "FF0B2026"
NAVY_800 = "FF1B3139"
NAVY_EDGE = "FF2A4A55"  # derived: lighter navy rim
OAT = "FFF9F7F4"
OAT_M = "FFEEEDE9"
REC_RED = "FFFF4D4D"
GOLD = "FFFFC94A"
GOLD_L = "FFFFF0B3"

MOOD_COLORS = {
    "idle": "FF8FF5FF",
    "listening": "FF7CFFB2",
    "processing": "FFB89CFF",
    "answering": "FFFFD166",
    "success": "FF6CFF8E",
    "error": "FFFF5A5A",
    "sleeping": "FF5C7C99",
}
MOODS = list(MOOD_COLORS)

# ---------------------------------------------------------------- ids
# The order of this list fixes the ids. "tail" is no longer drawn (U14), but its
# slot stays so that every later id (and the mood enum values) keeps its value.
I = {k: nid() for k in [
    "artboard", "abstyle", "sm", "vm", "vmi", "enum",
    "p_mood", "p_level", "p_hover", "p_poke", "p_demo",
    "mog", "squash", "facehover", "moodbody",
    "sparkle", "sparkle_shape", "orbit", "burst", "zz",
    "armL", "armR", "eyeslook", "lids", "eyepair",
    "eyeL_col", "eyeR_col", "happy", "happy_col", "xeyes", "xeyes_col",
    "sleepy", "sleepy_col", "mouth", "mouth_col", "thinkdots",
    "dot1", "dot2", "dot3", "dots_col1", "dots_col2", "dots_col3",
    "screen_glow", "torso", "tail", "rings", "levelring", "levelglow",
    "shadow", "conv_level", "ev_poked",
]}
ENUMV = {m: nid() for m in MOODS}
# New ids are allocated after the old ones so the existing ids stay unchanged.
I.update({k: nid() for k in [
    "p_attention", "p_recording", "attn", "rec", "recdot", "lamp", "lampglow",
]})

# ---------------------------------------------------------------- helpers


def fill(color, cid=None, name="Fill"):
    idattr = f' id="{cid}"' if cid else ""
    return f'<Fill name="{name}"><SolidColor colorValue="{color}" name="Color"{idattr}/></Fill>'


def stroke(color, thick, cid=None, name="Stroke", feather=None):
    idattr = f' id="{cid}"' if cid else ""
    f = f'<Feather strength="{feather}" name="Glow"/>' if feather else ""
    return (f'<Stroke thickness="{thick}" cap="round" join="round" name="{name}">'
            f'<SolidColor colorValue="{color}" name="Color"{idattr}/>{f}</Stroke>')


def rrect(w, h, r, name="Path", ox=0.5, oy=0.5):
    return (f'<Rectangle width="{w}" height="{h}" originX="{ox}" originY="{oy}" '
            f'cornerRadiusTL="{r}" name="{name}"/>')


def ellipse(w, h, name="Path"):
    return f'<Ellipse width="{w}" height="{h}" originX="0.5" originY="0.5" name="{name}"/>'


def polyline(pts, closed=False, radius=0, name="Path"):
    vs = "".join(f'<StraightVertex x="{x}" y="{y}" radius="{radius}"/>' for x, y in pts)
    return f'<PointsPath isClosed="{"true" if closed else "false"}" name="{name}">{vs}</PointsPath>'


def star4(w, cid=None, name="Sparkle"):
    return f'<Star width="{w}" height="{w}" points="4" innerRadius="0.32" cornerRadius="1.5" originX="0.5" originY="0.5" name="{name}"/>'


def bind(path_prop, key, converter=None, write=False):
    c = f' converterId="{converter}"' if converter else ""
    d = ' direction="true"' if write else ""
    return f'<DataBindContext sourcePathIds="{I["vm"]}-{path_prop}" propertyKey="{key}"{c}{d}/>'


# ---------------------------------------------------------------- scene
def sparkle_group():
    # Floating genie "jewel" above the head, plus a halo glow.
    jewel = (
        f'<Node x="0" y="-124" name="Sparkle" id="{I["sparkle"]}">'
        f'<Shape name="Jewel" id="{I["sparkle_shape"]}">{star4(34)}'
        f'<Fill name="Fill"><RadialGradient startX="0" startY="0" endX="17" endY="0" name="G">'
        f'<GradientStop colorValue="{GOLD_L}" position="0"/><GradientStop colorValue="{GOLD}" position="1"/>'
        f'</RadialGradient></Fill></Shape>'
        f'<Shape name="JewelGlow">{ellipse(40, 40)}'
        f'<Fill fillRule="clockwise" name="Glow"><SolidColor colorValue="88FFC94A" name="Color"/>'
        f'<Feather strength="14" name="F"/></Fill></Shape>'
        f'</Node>')
    # Orbiting sparkles shown while processing.
    orb = []
    for i, (r, s) in enumerate([(132, 16), (132, 11), (132, 8)]):
        a = i * TAU / 3 - math.pi / 2
        orb.append(f'<Shape x="{r*math.cos(a):.2f}" y="{r*math.sin(a):.2f}" name="Orb{i+1}">{star4(s)}{fill(GOLD)}</Shape>')
    orbit = f'<Node x="0" y="-6" opacity="0" name="Orbit" id="{I["orbit"]}">{"".join(orb)}</Node>'
    # Success burst: ring of small sparkles + confetti dots.
    b = []
    for i in range(8):
        a = i * TAU / 8
        r = 128 if i % 2 == 0 else 112
        sz = 14 if i % 2 == 0 else 8
        col = GOLD if i % 2 == 0 else "FF6CFF8E"
        b.append(f'<Shape x="{r*math.cos(a):.2f}" y="{r*math.sin(a):.2f}" name="Burst{i+1}">{star4(sz)}{fill(col)}</Shape>')
    burst = f'<Node x="0" y="-10" opacity="0" name="Burst" id="{I["burst"]}">{"".join(b)}</Node>'
    # Sleeping Zz.
    z = lambda s: polyline([(-s, -s), (s, -s), (-s, s), (s, s)])
    zz = (f'<Node x="78" y="-96" opacity="0" name="Zz" id="{I["zz"]}">'
          f'<Shape x="0" y="0" name="Z1">{z(9)}{stroke("FFB9D7EA", 4)}</Shape>'
          f'<Shape x="24" y="-26" name="Z2">{z(6)}{stroke("FFB9D7EA", 3)}</Shape>'
          f'</Node>')
    return jewel + orbit + burst + zz


def face():
    eye = lambda x, cid: (f'<Shape x="{x}" y="0" name="Eye">{rrect(24, 34, 12)}'
                          f'<Fill name="Fill"><SolidColor colorValue="{MOOD_COLORS["idle"]}" name="Color" id="{cid}"/></Fill>'
                          f'</Shape>')
    eyepair = (f'<Node name="EyePair" id="{I["eyepair"]}">'
               f'<Node name="Lids" id="{I["lids"]}">{eye(-30, I["eyeL_col"])}{eye(30, I["eyeR_col"])}</Node>'
               f'</Node>')
    happy = (f'<Shape opacity="0" name="HappyEyes" id="{I["happy"]}">'
             f'{polyline([(-42, 6), (-30, -7), (-18, 6)], radius=8, name="L")}'
             f'{polyline([(18, 6), (30, -7), (42, 6)], radius=8, name="R")}'
             f'<Stroke thickness="6" cap="round" join="round" name="Stroke"><SolidColor colorValue="{MOOD_COLORS["success"]}" name="Color" id="{I["happy_col"]}"/></Stroke>'
             f'</Shape>')
    xeyes = (f'<Shape opacity="0" name="XEyes" id="{I["xeyes"]}">'
             f'{polyline([(-39, -9), (-21, 9)], name="L1")}{polyline([(-21, -9), (-39, 9)], name="L2")}'
             f'{polyline([(21, -9), (39, 9)], name="R1")}{polyline([(39, -9), (21, 9)], name="R2")}'
             f'<Stroke thickness="6" cap="round" join="round" name="Stroke"><SolidColor colorValue="{MOOD_COLORS["error"]}" name="Color" id="{I["xeyes_col"]}"/></Stroke>'
             f'</Shape>')
    sleepy = (f'<Shape opacity="0" name="SleepEyes" id="{I["sleepy"]}">'
              f'{polyline([(-42, 0), (-30, 5), (-18, 0)], radius=8, name="L")}'
              f'{polyline([(18, 0), (30, 5), (42, 0)], radius=8, name="R")}'
              f'<Stroke thickness="5" cap="round" join="round" name="Stroke"><SolidColor colorValue="{MOOD_COLORS["sleeping"]}" name="Color" id="{I["sleepy_col"]}"/></Stroke>'
              f'</Shape>')
    look = f'<Node x="0" y="-6" name="EyesLook" id="{I["eyeslook"]}">{eyepair}{happy}{xeyes}{sleepy}</Node>'
    mouth = (f'<Shape x="0" y="24" name="Mouth" id="{I["mouth"]}">{rrect(18, 6, 3)}'
             f'<Fill name="Fill"><SolidColor colorValue="{MOOD_COLORS["idle"]}" name="Color" id="{I["mouth_col"]}"/></Fill></Shape>')
    dots = []
    for i, (x, did, cid) in enumerate([(-16, I["dot1"], I["dots_col1"]), (0, I["dot2"], I["dots_col2"]), (16, I["dot3"], I["dots_col3"])]):
        dots.append(f'<Shape x="{x}" y="0" name="Dot{i+1}" id="{did}">{ellipse(8, 8)}'
                    f'<Fill name="Fill"><SolidColor colorValue="{MOOD_COLORS["processing"]}" name="Color" id="{cid}"/></Fill></Shape>')
    think = f'<Node x="0" y="26" opacity="0" name="ThinkDots" id="{I["thinkdots"]}">{"".join(dots)}</Node>'
    screen = (f'<Shape x="0" y="0" name="Screen">{rrect(130, 88, 38)}'
              f'<Stroke thickness="3" name="Rim"><SolidColor colorValue="{NAVY_EDGE}" name="Color"/></Stroke>'
              f'<Fill name="Fill"><LinearGradient startX="0" startY="-44" endX="0" endY="44" name="G">'
              f'<GradientStop colorValue="{NAVY_800}" position="0"/><GradientStop colorValue="{NAVY}" position="1"/>'
              f'</LinearGradient></Fill></Shape>')
    glow = (f'<Shape x="0" y="0" name="ScreenGlow">{rrect(134, 92, 40)}'
            f'<Stroke thickness="4" name="Glow"><SolidColor colorValue="408FF5FF" name="Color" id="{I["screen_glow"]}"/>'
            f'<Feather strength="6" name="F"/></Stroke></Shape>')
    return (f'<Node x="0" y="-6" name="FaceHover" id="{I["facehover"]}">'
            f'{look}{mouth}{think}{screen}{glow}</Node>')


def arm(side, aid):
    x = -94 if side == "L" else 94
    return (f'<Node x="{x}" y="22" name="Arm{side}" id="{aid}">'
            f'<Shape x="0" y="18" name="Nub">{rrect(24, 42, 12)}'
            f'<Fill name="Fill"><LinearGradient startX="0" startY="-21" endX="0" endY="21" name="G">'
            f'<GradientStop colorValue="{PINK}" position="0"/><GradientStop colorValue="{CORAL}" position="1"/>'
            f'</LinearGradient></Fill></Shape></Node>')


def body():
    # Soft genie body: pale pink at the top, Genie coral where it rises out of the lamp.
    # The bottom of the body sits inside the lamp; the lamp front hides it.
    cheeks = "".join(
        f'<Shape x="{x}" y="50" name="Cheek">{ellipse(24, 13)}'
        f'<Fill fillRule="clockwise" name="Fill"><SolidColor colorValue="DDFF7B6B" name="Color"/>'
        f'<Feather strength="3" name="F"/></Fill></Shape>'
        for x in (-58, 58))
    torso = (f'<Shape x="0" y="22" name="Body" id="{I["torso"]}">{rrect(188, 186, 86)}'
             f'<Fill name="Fill"><LinearGradient startX="0" startY="-93" endX="0" endY="93" name="G">'
             f'<GradientStop colorValue="{PINK}" position="0.1"/><GradientStop colorValue="FFFFA898" position="0.6"/>'
             f'<GradientStop colorValue="{CORAL}" position="1"/></LinearGradient></Fill></Shape>')
    shine = f'<Shape x="-62" y="-44" name="Shine">{ellipse(18, 11)}{fill("88FFFFFF")}</Shape>'
    return (f'<Node name="MoodBody" id="{I["moodbody"]}">'
            f'{arm("L", I["armL"])}{arm("R", I["armR"])}{face()}{cheeks}{shine}{torso}</Node>')


def lamp_front():
    # Genie lamp, an original simplified drawing that extends the Genie One lamp icon.
    # Local origin = the lamp mouth. Drawn in front of the genie.
    lip = (f'<Shape x="0" y="0" name="Lip">{rrect(214, 18, 9)}'
           f'<Fill name="Fill"><LinearGradient startX="0" startY="-9" endX="0" endY="9" name="G">'
           f'<GradientStop colorValue="{OAT}" position="0"/><GradientStop colorValue="{OAT_M}" position="1"/>'
           f'</LinearGradient></Fill></Shape>')
    gleam = (f'<Shape x="-60" y="28" name="Gleam">{rrect(40, 9, 4.5)}'
             f'<Fill name="Fill"><SolidColor colorValue="AAFABFBA" name="Color"/></Fill></Shape>'
             f'<Shape x="64" y="30" name="LampSparkle">{star4(14)}{fill(PINK)}</Shape>')
    bowl = (f'<Shape x="0" y="0" name="Bowl">'
            f'{polyline([(-112, 0), (112, 0), (88, 66), (-88, 66)], closed=True, radius=36)}'
            f'<Fill name="Fill"><LinearGradient startX="0" startY="0" endX="0" endY="66" name="G">'
            f'<GradientStop colorValue="{CORAL}" position="0"/><GradientStop colorValue="{CORAL_D}" position="1"/>'
            f'</LinearGradient></Fill></Shape>')
    spout = (f'<Shape x="0" y="0" name="Spout">'
             f'{polyline([(92, 10), (132, 6), (156, -14), (172, -26), (166, -8), (136, 30), (82, 46)], closed=True, radius=9)}'
             f'<Fill name="Fill"><LinearGradient startX="80" startY="0" endX="166" endY="-24" name="G">'
             f'<GradientStop colorValue="{CORAL_D}" position="0"/><GradientStop colorValue="{CORAL}" position="1"/>'
             f'</LinearGradient></Fill></Shape>')
    handle = (f'<Shape x="-118" y="26" name="Handle">{ellipse(52, 46)}'
              f'{stroke(CORAL_DD, 11)}</Shape>')
    neck = f'<Shape x="0" y="72" name="Neck">{rrect(70, 14, 5)}{fill(CORAL_DD)}</Shape>'
    foot = (f'<Shape x="0" y="84" name="Foot">{rrect(122, 16, 8)}'
            f'<Fill name="Fill"><LinearGradient startX="0" startY="-8" endX="0" endY="8" name="G">'
            f'<GradientStop colorValue="{OAT_M}" position="0"/><GradientStop colorValue="FFD8D5CE" position="1"/>'
            f'</LinearGradient></Fill></Shape>')
    return (f'<Node x="200" y="276" name="Lamp" id="{I["lamp"]}">'
            f'{lip}{gleam}{bowl}{spout}{handle}{neck}{foot}</Node>')


def lamp_inside():
    # The dark mouth of the lamp, drawn behind the genie, with a soft coral glow.
    return (f'<Node x="200" y="270" name="LampInside">'
            f'<Shape name="Mouth">{ellipse(200, 34)}{fill(NAVY)}</Shape>'
            f'<Shape name="MouthGlow" id="{I["lampglow"]}">{ellipse(170, 40)}'
            f'<Fill fillRule="clockwise" name="Glow"><SolidColor colorValue="88FF5F46" name="Color"/>'
            f'<Feather strength="16" name="F"/></Fill></Shape></Node>')


def attention():
    # "?" bubble (UX-03), shown over any mood when attention == true.
    q = (f'<Shape x="0" y="-3" name="Hook">'
         f'{polyline([(-7, -5), (-7, -10), (0, -14), (7, -10), (7, -4), (0, 1), (0, 6)], radius=5)}'
         f'{stroke(NAVY_800, 5)}</Shape>'
         f'<Shape x="0" y="13" name="Dot">{ellipse(6, 6)}{fill(NAVY_800)}</Shape>')
    bubble = (f'<Shape name="Bubble">{ellipse(48, 48)}'
              f'<Stroke thickness="3" name="Rim"><SolidColor colorValue="{CORAL}" name="Color"/></Stroke>'
              f'{fill(OAT)}</Shape>'
              f'<Shape x="-16" y="20" name="BubbleTail">{ellipse(12, 12)}'
              f'<Stroke thickness="3" name="Rim"><SolidColor colorValue="{CORAL}" name="Color"/></Stroke>{fill(OAT)}</Shape>')
    return f'<Node x="104" y="-112" opacity="0" name="Attention" id="{I["attn"]}">{q}{bubble}</Node>'


def recording():
    # Red ring and REC dot (UX-04a). The DOM REC pill stays the authoritative indicator (UX-04b).
    dot = (f'<Shape x="-112" y="-118" name="RecDot" id="{I["recdot"]}">{ellipse(18, 18)}{fill(REC_RED)}</Shape>')
    ring = (f'<Shape name="RecRing">{ellipse(292, 292)}{stroke("DDFF4D4D", 4)}</Shape>'
            f'<Shape name="RecGlow">{ellipse(292, 292)}{stroke("66FF4D4D", 6, feather=8)}</Shape>')
    return f'<Node x="0" y="-14" opacity="0" name="Recording" id="{I["rec"]}">{dot}{ring}</Node>'


def rings():
    r = lambda w, a, t: f'<Shape name="Ring">{ellipse(w, w)}{stroke(a, t)}</Shape>'
    return (f'<Node x="0" y="-16" opacity="0" name="ListenRings" id="{I["rings"]}">'
            f'<Node name="LevelRing" id="{I["levelring"]}">'
            f'<DataBindContext sourcePathIds="{I["vm"]}-{I["p_level"]}" propertyKey="16" converterId="{I["conv_level"]}"/>'
            f'<DataBindContext sourcePathIds="{I["vm"]}-{I["p_level"]}" propertyKey="17" converterId="{I["conv_level"]}"/>'
            f'{r(250, "AA7CFFB2", 4)}'
            f'<Node name="LevelGlow" id="{I["levelglow"]}">'
            f'<DataBindContext sourcePathIds="{I["vm"]}-{I["p_level"]}" propertyKey="18"/>'
            f'<Shape name="GlowRing">{ellipse(250, 250)}{stroke("AA7CFFB2", 5, feather=8)}</Shape></Node>'
            f'</Node>'
            f'{r(300, "557CFFB2", 3)}</Node>')


def scene_tree():
    # Front to back: lamp front, genie (floats), lamp inside, shadow.
    mog = (f'<Node x="200" y="186" name="Mog" id="{I["mog"]}">'
           f'{attention()}{recording()}{sparkle_group()}'
           f'<Node name="Squash" id="{I["squash"]}">{body()}</Node>'
           f'{rings()}</Node>')
    shadow = (f'<Shape x="200" y="378" name="Shadow" id="{I["shadow"]}">{ellipse(150, 16)}'
              f'<Fill fillRule="clockwise" name="Fill"><SolidColor colorValue="44000000" name="Color"/>'
              f'<Feather strength="8" name="F"/></Fill></Shape>')
    return lamp_front() + mog + lamp_inside() + shadow


# ---------------------------------------------------------------- animation
# property -> (objectId, propertyName, kind)
P = {
    "lookX": (I["eyeslook"], "x", "d"), "lookY": (I["eyeslook"], "y", "d"),
    "eyeSX": (I["eyepair"], "scaleX", "d"), "eyeSY": (I["eyepair"], "scaleY", "d"),
    "eyeO": (I["eyepair"], "opacity", "d"),
    "happyO": (I["happy"], "opacity", "d"), "xO": (I["xeyes"], "opacity", "d"),
    "sleepO": (I["sleepy"], "opacity", "d"),
    "mouthSX": (I["mouth"], "scaleX", "d"), "mouthSY": (I["mouth"], "scaleY", "d"),
    "mouthO": (I["mouth"], "opacity", "d"),
    "thinkO": (I["thinkdots"], "opacity", "d"),
    "dot1Y": (I["dot1"], "y", "d"), "dot2Y": (I["dot2"], "y", "d"), "dot3Y": (I["dot3"], "y", "d"),
    "armL": (I["armL"], "rotation", "d"), "armR": (I["armR"], "rotation", "d"),
    "bodyX": (I["moodbody"], "x", "d"), "bodyY": (I["moodbody"], "y", "d"),
    "bodyR": (I["moodbody"], "rotation", "d"), "bodySY": (I["moodbody"], "scaleY", "d"),
    "spY": (I["sparkle"], "y", "d"), "spR": (I["sparkle"], "rotation", "d"),
    "spSX": (I["sparkle"], "scaleX", "d"), "spSY": (I["sparkle"], "scaleY", "d"),
    "spO": (I["sparkle"], "opacity", "d"),
    "orbO": (I["orbit"], "opacity", "d"), "orbR": (I["orbit"], "rotation", "d"),
    "burstO": (I["burst"], "opacity", "d"), "burstS": (I["burst"], "scaleX", "d"),
    "burstS2": (I["burst"], "scaleY", "d"), "burstR": (I["burst"], "rotation", "d"),
    "zzO": (I["zz"], "opacity", "d"), "zzY": (I["zz"], "y", "d"),
    "ringsO": (I["rings"], "opacity", "d"), "ringsS": (I["rings"], "scaleX", "d"),
    "ringsS2": (I["rings"], "scaleY", "d"),
    "colL": (I["eyeL_col"], "colorValue", "c"), "colR": (I["eyeR_col"], "colorValue", "c"),
    "colM": (I["mouth_col"], "colorValue", "c"), "colG": (I["screen_glow"], "colorValue", "c"),
}

BASE = {
    "lookX": 0, "lookY": -6, "eyeSX": 1, "eyeSY": 1, "eyeO": 1, "happyO": 0, "xO": 0, "sleepO": 0,
    "mouthSX": 1, "mouthSY": 1, "mouthO": 1, "thinkO": 0, "dot1Y": 0, "dot2Y": 0, "dot3Y": 0,
    "armL": 0.12, "armR": -0.12, "bodyX": 0, "bodyY": 0, "bodyR": 0, "bodySY": 1,
    "spY": -124, "spR": 0, "spSX": 1, "spSY": 1, "spO": 1, "orbO": 0, "orbR": 0,
    "burstO": 0, "burstS": 0.6, "burstS2": 0.6, "burstR": 0, "zzO": 0, "zzY": -96,
    "ringsO": 0, "ringsS": 1, "ringsS2": 1,
}


def glow(c):
    return "40" + c[2:]


def mood_spec(m):
    c = MOOD_COLORS[m]
    s = dict(BASE)
    s.update(colL=c, colR=c, colM=c, colG=glow(c))
    D = 120
    loop = "loop"
    if m == "idle":
        D = 180
        s.update(spY=[(0, -124), (90, -132), (180, -124)], spR=[(0, -0.15), (90, 0.15), (180, -0.15)],
                 armL=[(0, 0.12), (90, 0.2), (180, 0.12)], armR=[(0, -0.12), (90, -0.2), (180, -0.12)])
    elif m == "listening":
        D = 90
        s.update(ringsO=1, ringsS=[(0, 0.96), (45, 1.06), (90, 0.96)], ringsS2=[(0, 0.96), (45, 1.06), (90, 0.96)],
                 eyeSX=1.12, eyeSY=1.18, lookY=-8, mouthSX=0.6, mouthSY=1.6,
                 armR=[(0, -2.3), (45, -2.45), (90, -2.3)], armL=0.25, bodyR=-0.04,
                 spY=-132, spSX=0.85, spSY=0.85)
    elif m == "processing":
        D = 120
        s.update(orbO=1, orbR=[(0, 0, "linear"), (120, TAU)], spR=[(0, 0, "linear"), (120, TAU)],
                 lookX=10, lookY=-12, eyeSX=0.9, eyeSY=0.55, mouthO=0, thinkO=1,
                 dot1Y=[(0, 0), (20, -7), (40, 0), (120, 0)],
                 dot2Y=[(0, 0), (20, 0), (40, -7), (60, 0), (120, 0)],
                 dot3Y=[(0, 0), (40, 0), (60, -7), (80, 0), (120, 0)],
                 armL=0.35, armR=-0.35, spY=-134)
    elif m == "answering":
        D = 60
        s.update(mouthSY=[(0, 1), (8, 3.2), (18, 1.5), (28, 2.8), (40, 1.2), (50, 2.4), (60, 1)],
                 mouthSX=[(0, 1), (8, 0.8), (28, 0.85), (60, 1)],
                 armR=[(0, -0.6), (30, -0.95), (60, -0.6)], armL=[(0, 0.15), (30, 0.3), (60, 0.15)],
                 spSX=[(0, 1.1), (30, 1.25), (60, 1.1)], spSY=[(0, 1.1), (30, 1.25), (60, 1.1)],
                 bodyR=[(0, 0.02), (30, -0.02), (60, 0.02)])
    elif m == "success":
        D = 90
        s.update(eyeO=0, happyO=1, burstO=1, burstS=[(0, 0.9), (45, 1.12), (90, 0.9)],
                 burstS2=[(0, 0.9), (45, 1.12), (90, 0.9)], burstR=[(0, 0, "linear"), (90, 0.785)],
                 armL=[(0, 2.5), (45, 2.75), (90, 2.5)], armR=[(0, -2.5), (45, -2.75), (90, -2.5)],
                 bodyY=[(0, 0), (22, -12), (45, 0), (90, 0)], mouthSX=1.6, mouthSY=1.4,
                 spY=-140, spSX=1.3, spSY=1.3)
    elif m == "error":
        D = 90
        s.update(eyeO=0, xO=1, mouthSX=1.3, mouthSY=0.8,
                 bodyX=[(0, 0), (5, -7), (10, 7), (15, -6), (20, 6), (25, -3), (30, 0), (90, 0)],
                 armL=0.02, armR=-0.02, spY=-110, spO=0.35, spR=0.6, bodyR=0.03)
    elif m == "sleeping":
        D = 240
        loop = "pingPong"
        s.update(eyeO=0, sleepO=1, mouthO=0.4, mouthSX=0.6, zzO=[(0, 0.3), (240, 1)], zzY=[(0, -92), (240, -112)],
                 bodySY=[(0, 1), (240, 0.97)], spY=-108, spO=0.35, spSX=0.8, spSY=0.8, armL=0.05, armR=-0.05,
                 lookY=2)
    return s, D, loop


def keyframes(kind, spec, D):
    if not isinstance(spec, list):
        spec = [(0, spec)]
    out = []
    for i, k in enumerate(spec):
        f, v = k[0], k[1]
        interp = k[2] if len(k) > 2 else ("cubic" if kind == "d" else "linear")
        if kind == "c":
            out.append(f'<KeyFrameColor value="{v}" frame="{f}" interpolationType="hold"/>')
        elif interp == "cubic" and i < len(spec) - 1:
            out.append(f'<KeyFrameDouble value="{v}" frame="{f}" interpolationType="cubic">{EASE}</KeyFrameDouble>')
        else:
            out.append(f'<KeyFrameDouble value="{v}" frame="{f}" interpolationType="{interp if i < len(spec) - 1 else "linear"}"/>')
    return "".join(out)


def anim(name, aid, D, loop, props):
    by_obj = {}
    for key, spec in props.items():
        oid, pname, kind = P[key]
        by_obj.setdefault(oid, []).append(
            f'<KeyedProperty property="{pname}">{keyframes(kind, spec, D)}</KeyedProperty>')
    body = "".join(f'<KeyedObject objectId="{o}">{"".join(v)}</KeyedObject>' for o, v in by_obj.items())
    return f'<LinearAnimation loopValue="{loop}" fps="60" duration="{D}" name="{name}" id="{aid}">{body}</LinearAnimation>'


def simple_anim(name, aid, D, loop, keyed):
    """keyed: list of (objectId, prop, [(frame, value[, interp])])"""
    by_obj = {}
    for oid, pname, spec in keyed:
        by_obj.setdefault(oid, []).append(
            f'<KeyedProperty property="{pname}">{keyframes("d", spec, D)}</KeyedProperty>')
    body = "".join(f'<KeyedObject objectId="{o}">{"".join(v)}</KeyedObject>' for o, v in by_obj.items())
    return f'<LinearAnimation loopValue="{loop}" fps="60" duration="{D}" name="{name}" id="{aid}">{body}</LinearAnimation>'


# ---------------------------------------------------------------- state machine
def cond_enum(mood):
    return (f'<TransitionViewModelCondition><TransitionPropertyViewModelComparator><BindablePropertyEnum>'
            f'{bind(I["p_mood"], 637)}</BindablePropertyEnum></TransitionPropertyViewModelComparator>'
            f'<TransitionValueEnumComparator value="{ENUMV[mood]}"/></TransitionViewModelCondition>')


def cond_bool(prop, val):
    return (f'<TransitionViewModelCondition><TransitionPropertyViewModelComparator><BindablePropertyBoolean>'
            f'{bind(prop, 634)}</BindablePropertyBoolean></TransitionPropertyViewModelComparator>'
            f'<TransitionValueBooleanComparator value="{val}"/></TransitionViewModelCondition>')


def cond_trigger(prop):
    return (f'<TransitionViewModelCondition><TransitionPropertyViewModelComparator><BindablePropertyTrigger>'
            f'{bind(prop, 686)}</BindablePropertyTrigger></TransitionPropertyViewModelComparator>'
            f'<TransitionValueTriggerComparator/></TransitionViewModelCondition>')


def write_mood(mood):
    return (f'<ListenerViewModelChange><BindablePropertyEnum propertyValue="{ENUMV[mood]}">'
            f'{bind(I["p_mood"], 637, write=True)}</BindablePropertyEnum></ListenerViewModelChange>')


def build():
    anims = []
    # Mood layer
    mood_state = {m: nid() for m in MOODS}
    mood_anim = {m: nid() for m in MOODS}
    for m in MOODS:
        s, D, loop = mood_spec(m)
        anims.append(anim(f"Mood {m}", mood_anim[m], D, loop, s))
    mood_states = []
    for i, m in enumerate(MOODS):
        trans = "".join(
            f'<StateTransition stateToId="{mood_state[t]}" duration="260" enableEarlyExit="true">{cond_enum(t)}</StateTransition>'
            for t in MOODS if t != m)
        mood_states.append(f'<AnimationState x="{160 + 200*i}" y="{0 if i % 2 == 0 else 160}" animationId="{mood_anim[m]}" id="{mood_state[m]}">{trans}</AnimationState>')
    mood_layer = (f'<StateMachineLayer name="Mood" id="{nid()}"><AnyState x="160" y="-160"/><ExitState x="1600" y="-160"/>'
                  f'<EntryState x="0" y="0"><StateTransition stateToId="{mood_state["idle"]}"/></EntryState>'
                  f'{"".join(mood_states)}</StateMachineLayer>')

    # Float layer (always on)
    a_float = nid()
    anims.append(simple_anim("Float", a_float, 150, "pingPong", [
        # The genie bobs inside the lamp; the lamp stays still and its mouth glows.
        (I["mog"], "y", [(0, 192), (150, 178)]),
        (I["lampglow"], "opacity", [(0, 0.7), (150, 1)]),
    ]))
    s_float = nid()
    float_layer = (f'<StateMachineLayer name="Float" id="{nid()}"><AnyState x="160" y="-160"/><ExitState x="400" y="-160"/>'
                   f'<EntryState><StateTransition stateToId="{s_float}"/></EntryState>'
                   f'<AnimationState x="160" animationId="{a_float}" id="{s_float}"/></StateMachineLayer>')

    # Blink layer
    a_blink = nid()
    anims.append(simple_anim("Blink", a_blink, 260, "loop", [
        (I["lids"], "scaleY", [(0, 1, "hold"), (220, 1), (226, 0.08), (232, 1, "hold"), (238, 1), (243, 0.08), (249, 1, "hold"), (260, 1)]),
    ]))
    s_blink = nid()
    blink_layer = (f'<StateMachineLayer name="Blink" id="{nid()}"><AnyState x="160" y="-160"/><ExitState x="400" y="-160"/>'
                   f'<EntryState><StateTransition stateToId="{s_blink}"/></EntryState>'
                   f'<AnimationState x="160" animationId="{a_blink}" id="{s_blink}"/></StateMachineLayer>')

    # Hover layer
    a_hov0, a_hov1 = nid(), nid()
    anims.append(simple_anim("Hover off", a_hov0, 1, "oneShot", [
        (I["facehover"], "scaleX", [(0, 1)]), (I["facehover"], "scaleY", [(0, 1)])]))
    anims.append(simple_anim("Hover on", a_hov1, 1, "oneShot", [
        (I["facehover"], "scaleX", [(0, 1.06)]), (I["facehover"], "scaleY", [(0, 1.06)])]))
    s_h0, s_h1 = nid(), nid()
    hover_layer = (f'<StateMachineLayer name="Hover" id="{nid()}"><AnyState x="160" y="-160"/><ExitState x="560" y="-160"/>'
                   f'<EntryState><StateTransition stateToId="{s_h0}"/></EntryState>'
                   f'<AnimationState x="160" animationId="{a_hov0}" id="{s_h0}">'
                   f'<StateTransition stateToId="{s_h1}" duration="160" enableEarlyExit="true">{cond_bool(I["p_hover"], "true")}</StateTransition></AnimationState>'
                   f'<AnimationState x="360" animationId="{a_hov1}" id="{s_h1}">'
                   f'<StateTransition stateToId="{s_h0}" duration="200" enableEarlyExit="true">{cond_bool(I["p_hover"], "false")}</StateTransition></AnimationState>'
                   f'</StateMachineLayer>')

    # Poke layer: squash & stretch bounce on click (and Event out to host)
    a_rest, a_poke = nid(), nid()
    anims.append(simple_anim("Poke rest", a_rest, 1, "oneShot", [
        (I["squash"], "scaleX", [(0, 1)]), (I["squash"], "scaleY", [(0, 1)]), (I["squash"], "y", [(0, 0)])]))
    anims.append(simple_anim("Poke bounce", a_poke, 40, "oneShot", [
        (I["squash"], "scaleX", [(0, 1), (6, 1.12), (16, 0.92), (26, 1.03), (40, 1)]),
        (I["squash"], "scaleY", [(0, 1), (6, 0.86), (16, 1.1), (26, 0.98), (40, 1)]),
        (I["squash"], "y", [(0, 0), (6, 8), (16, -16), (28, 0), (40, 0)]),
    ]))
    s_rest, s_poke = nid(), nid()
    poke_layer = (f'<StateMachineLayer name="Poke" id="{nid()}"><AnyState x="160" y="-160"/><ExitState x="560" y="-160"/>'
                  f'<EntryState><StateTransition stateToId="{s_rest}"/></EntryState>'
                  f'<AnimationState x="160" animationId="{a_rest}" id="{s_rest}">'
                  f'<StateTransition stateToId="{s_poke}">{cond_trigger(I["p_poke"])}</StateTransition></AnimationState>'
                  f'<AnimationState x="360" animationId="{a_poke}" reset="true" id="{s_poke}">'
                  f'<StateMachineFireEvent eventId="{I["ev_poked"]}" occursValue="atStart"/>'
                  f'<StateTransition stateToId="{s_rest}" enableExitTime="true" exitTimeIsPercetange="true" exitTime="100"/>'
                  f'<StateTransition stateToId="{s_poke}">{cond_trigger(I["p_poke"])}</StateTransition>'
                  f'</AnimationState></StateMachineLayer>')

    # Demo layer: click cycles mood while demo == true
    a_tick = nid()
    anims.append(f'<LinearAnimation loopValue="oneShot" fps="60" duration="1" name="Demo tick" id="{a_tick}"/>')
    cycle = {"idle": "listening", "listening": "processing", "processing": "answering",
             "answering": "success", "success": "idle", "error": "idle", "sleeping": "idle"}
    s_wait = nid()
    set_states = {}
    any_trans = []
    sets = []
    for i, (frm, to) in enumerate(cycle.items()):
        sid = nid()
        set_states[frm] = sid
        any_trans.append(f'<StateTransition stateToId="{sid}">{cond_trigger(I["p_poke"])}{cond_bool(I["p_demo"], "true")}{cond_enum(frm)}</StateTransition>')
        sets.append(f'<AnimationState x="{160 + 200*i}" y="200" animationId="{a_tick}" id="{sid}">{write_mood(to)}</AnimationState>')
    demo_layer = (f'<StateMachineLayer name="Demo" id="{nid()}"><AnyState x="560" y="-160">{"".join(any_trans)}</AnyState>'
                  f'<ExitState x="1600" y="-160"/>'
                  f'<EntryState><StateTransition stateToId="{s_wait}"/></EntryState>'
                  f'<AnimationState x="160" animationId="{a_tick}" id="{s_wait}"/>{"".join(sets)}</StateMachineLayer>')

    # Overlay layers (D64): orthogonal to the mood, driven by booleans.
    def overlay_layer(name, prop, node, on_keys, on_dur):
        a_off, a_on = nid(), nid()
        anims.append(simple_anim(f"{name} off", a_off, 1, "oneShot", [(node, "opacity", [(0, 0)])]))
        anims.append(simple_anim(f"{name} on", a_on, on_dur, "loop", [(node, "opacity", [(0, 1)])] + on_keys))
        s0, s1 = nid(), nid()
        return (f'<StateMachineLayer name="{name}" id="{nid()}"><AnyState x="160" y="-160"/><ExitState x="560" y="-160"/>'
                f'<EntryState><StateTransition stateToId="{s0}"/></EntryState>'
                f'<AnimationState x="160" animationId="{a_off}" id="{s0}">'
                f'<StateTransition stateToId="{s1}" duration="160" enableEarlyExit="true">{cond_bool(prop, "true")}</StateTransition></AnimationState>'
                f'<AnimationState x="360" animationId="{a_on}" id="{s1}">'
                f'<StateTransition stateToId="{s0}" duration="160" enableEarlyExit="true">{cond_bool(prop, "false")}</StateTransition></AnimationState>'
                f'</StateMachineLayer>')

    attn_layer = overlay_layer("Attention", I["p_attention"], I["attn"],
                               [(I["attn"], "y", [(0, -112), (45, -120), (90, -112)])], 90)
    rec_layer = overlay_layer("Recording", I["p_recording"], I["rec"],
                              [(I["recdot"], "opacity", [(0, 1), (40, 0.25), (80, 1)])], 80)

    def lst(kind, name, action):
        return f'<StateMachineListenerSingle targetId="{I["torso"]}" listenerTypeValue="{kind}" name="{name}">{action}</StateMachineListenerSingle>'

    wbool = lambda p, v: (f'<ListenerViewModelChange><BindablePropertyBoolean propertyValue="{v}">'
                          f'{bind(p, 634, write=True)}</BindablePropertyBoolean></ListenerViewModelChange>')
    listeners = (lst("enter", "Hover in", wbool(I["p_hover"], "true")) +
                 lst("exit", "Hover out", wbool(I["p_hover"], "false")) +
                 lst("click", "Poke", f'<ListenerViewModelChange><BindablePropertyTrigger propertyValue="1">'
                                      f'{bind(I["p_poke"], 686, write=True)}</BindablePropertyTrigger></ListenerViewModelChange>'))

    sm = (f'<StateMachine name="Mog" id="{I["sm"]}">{listeners}'
          f'{float_layer}{blink_layer}{mood_layer}{hover_layer}{poke_layer}{demo_layer}{attn_layer}{rec_layer}</StateMachine>')

    artboard = (f'<Artboard defaultStateMachineId="{I["sm"]}" viewModelId="{I["vm"]}" viewModelInstanceId="{I["vmi"]}" '
                f'styleId="{I["abstyle"]}" width="400" height="400" clip="false" name="Mog" id="{I["artboard"]}">'
                f'<LayoutComponentStyle name="Artboard Style" id="{I["abstyle"]}"/>'
                f'<Event name="poked" id="{I["ev_poked"]}"/>'
                f'{scene_tree()}{sm}{"".join(anims)}</Artboard>')

    enum = (f'<DataEnumCustom name="MogMood" id="{I["enum"]}">' +
            "".join(f'<DataEnumValue key="{m}" value="{m.capitalize()}" id="{ENUMV[m]}"/>' for m in MOODS) +
            '</DataEnumCustom>')
    conv = (f'<DataConverterRangeMapper minInput="0" maxInput="1" minOutput="0.92" maxOutput="1.28" '
            f'clampLower="true" clampUpper="true" name="LevelToScale" id="{I["conv_level"]}"/>')
    vm = (f'<ViewModel defaultInstanceId="{I["vmi"]}" name="Mog" id="{I["vm"]}">'
          f'<ViewModelPropertyEnumCustom enumId="{I["enum"]}" name="mood" id="{I["p_mood"]}"/>'
          f'<ViewModelPropertyNumber name="level" id="{I["p_level"]}"/>'
          f'<ViewModelPropertyBoolean name="hover" id="{I["p_hover"]}"/>'
          f'<ViewModelPropertyTrigger name="poke" id="{I["p_poke"]}"/>'
          f'<ViewModelPropertyBoolean name="demo" id="{I["p_demo"]}"/>'
          f'<ViewModelPropertyBoolean name="attention" id="{I["p_attention"]}"/>'
          f'<ViewModelPropertyBoolean name="recording" id="{I["p_recording"]}"/>'
          f'<ViewModelInstance exports="true" name="Default" id="{I["vmi"]}">'
          f'<ViewModelInstanceEnum propertyValue="{ENUMV["idle"]}" viewModelPropertyId="{I["p_mood"]}"/>'
          f'<ViewModelInstanceNumber propertyValue="0" viewModelPropertyId="{I["p_level"]}"/>'
          f'<ViewModelInstanceBoolean propertyValue="false" viewModelPropertyId="{I["p_hover"]}"/>'
          f'<ViewModelInstanceTrigger viewModelPropertyId="{I["p_poke"]}"/>'
          f'<ViewModelInstanceBoolean propertyValue="false" viewModelPropertyId="{I["p_demo"]}"/>'
          f'<ViewModelInstanceBoolean propertyValue="false" viewModelPropertyId="{I["p_attention"]}"/>'
          f'<ViewModelInstanceBoolean propertyValue="false" viewModelPropertyId="{I["p_recording"]}"/>'
          f'</ViewModelInstance></ViewModel>')
    return f'<Rive version="1" kind="fragment">\n{artboard}\n{enum}\n{conv}\n{vm}\n</Rive>\n'


if __name__ == "__main__":
    xml = build()
    try:
        import xml.dom.minidom as md
        xml = md.parseString(xml).toprettyxml(indent="    ").split("\n", 1)[1]
    except Exception as e:  # keep raw if pretty-print fails
        print("pretty-print skipped:", e)
    with open(OUT, "w") as f:
        f.write(xml)
    print("wrote", os.path.abspath(OUT))
