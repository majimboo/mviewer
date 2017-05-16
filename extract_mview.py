#!/usr/bin/python
# @majidemo

import os, io, sys, struct

def main(filename):
    folder = filename.split(".")[0]
    mkDIR(folder)

    bin = load(filename)

    bin.seek(0, 2)
    end = bin.tell()
    bin.seek(0)

    while bin.tell() < end:
        name = readcstr(bin)
        ftype = readcstr(bin)
        c = readuint32(bin)
        d = readuint32(bin)
        e = readuint32(bin)

        data = bin.read(d)

        if c & 1:
            data = decompress(data, e)

        output = open("%s/%s" % (folder, name), "wb")
        output.write(data)
        output.close()

        print(name, ftype)

    print("COMPLETED!!!")

def decompress(a, b):
    c = bytearray(b)
    d = 0
    e = [0] * 4096
    f = [0] * 4096
    g = 256
    h = len(a)
    k = 0
    l = 1
    m = 0
    n = 1

    c[d] = a[0]
    d += 1

    r = 1
    while True:
        n = r + (r >> 1)
        if (n + 1) >= h:
            break
        m = a[n + 1]
        n = a[n]
        p = (m << 4 | n >> 4) if r & 1 else ((m & 15) << 8 | n)
        if p < g:
            if 256 > p:
                m = d
                n = 1
                c[d] = p
                d += 1
            else:
                m = d
                n = f[p]
                p = e[p]
                q = p + n
                while p < q:
                    c[d] = c[p]
                    d += 1
                    p += 1
        elif p == g:
            m = d
            n = l + 1
            p = k
            q = k + l
            while p < q:
                c[d] = c[p]
                d += 1
                p += 1
            c[d] = c[k]
            d += 1
        else:
            break

        e[g] = k
        f[g] = l + 1
        g += 1
        k = m
        l = n
        g = 256 if 4096 <= g else g
        r += 1

    return c if d == b else None

def readuint32(f):
    return struct.unpack("<I", f.read(4))[0]

def readcstr(f):
    buf = []
    while True:
        b = struct.unpack("<b", f.read(1))[0]
        if b == 0:
            return "".join(map(chr, buf))
        else:
            buf.append(b)

def mkDIR(dir):
    if not os.path.exists(dir):
        os.makedirs(dir)

def load(file):
    return open(file, "rb")

main(sys.argv[1])