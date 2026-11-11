#!/usr/bin/env /usr/bin/python3 -I
"""Generate the placeholder OMT app icon (128x128 RGBA PNG): an isometric
grass-block cube on a dark rounded square. Pure stdlib PNG writer; replace
with a designed icon whenever one exists."""
import struct, zlib

S = 128
px = [[(0, 0, 0, 0)] * S for _ in range(S)]

# rounded-square backdrop (dark slate)
def inside_round(x, y, r=18):
    if x < r and y < r and (x - r) ** 2 + (y - r) ** 2 > r * r: return False
    if x >= S - r and y < r and (x - (S - 1 - r)) ** 2 + (y - r) ** 2 > r * r: return False
    if x < r and y >= S - r and (x - r) ** 2 + (y - (S - 1 - r)) ** 2 > r * r: return False
    if x >= S - r and y >= S - r and (x - (S - 1 - r)) ** 2 + (y - (S - 1 - r)) ** 2 > r * r: return False
    return True

for y in range(S):
    for x in range(S):
        if inside_round(x, y):
            px[y][x] = (34, 39, 46, 255)

# isometric cube, centered; faces via half-plane tests in cube space
cx, cy = S / 2, S / 2 + 2
w, h = 36.0, 18.0      # half-width / half-height of the top rhombus
depth = 26.0           # vertical drop of the side faces
top = (146, 208, 90)
left = (106, 160, 62)
right = (78, 126, 48)

for y in range(S):
    for x in range(S):
        dx, dy = x - cx, y - cy
        # top face: |dy + dx*h/w| <= h and |dy - dx*h/w| <= h, dy <= 0 region
        a = abs(dy + dx * h / w) <= h
        b = abs(dy - dx * h / w) <= h
        if a and b and dy <= 0:
            px[y][x] = top + (255,)
        elif dy >= 0 and dy <= depth:
            # left face: dy+dx*h/w within [0..depth-slope] band on the left half
            if dx <= 0 and abs(dy + dx * h / w - 0) <= h and -w <= dx <= 0:
                px[y][x] = left + (255,)
            elif dx > 0 and abs(dy - dx * h / w) <= h and 0 <= dx <= w:
                px[y][x] = right + (255,)

raw = b''.join(b'\x00' + b''.join(struct.pack('4B', *p) for p in row) for row in px)

def chunk(tag, data):
    return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data))

png = (b'\x89PNG\r\n\x1a\n'
       + chunk(b'IHDR', struct.pack('>IIBBBBB', S, S, 8, 6, 0, 0, 0))
       + chunk(b'IDAT', zlib.compress(raw, 9))
       + chunk(b'IEND', b''))
open('/mnt/dev/Dev/Reverse_Engineering/UMT_Crack/Source/OmniMCTools/packaging/linux/omt.png', 'wb').write(png)
print('icon written', len(png), 'bytes')
