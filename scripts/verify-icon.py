"""Verify grayscale pixels in iconutil-decoded representations, including thumbnails."""
from pathlib import Path
import struct,zlib
import sys
root=Path(sys.argv[1])

def pixels(path):
 data=path.read_bytes(); p=8; payload=b''
 while p<len(data):
  n=struct.unpack('>I',data[p:p+4])[0]; kind=data[p+4:p+8]; block=data[p+8:p+8+n]; p+=n+12
  if kind==b'IHDR': w,h,depth,color,_,_,interlace=struct.unpack('>IIBBBBB',block)
  if kind==b'IDAT': payload+=block
 assert depth==8 and interlace==0 and color in (2,6), (path,color)
 channels={2:3,6:4}[color]; stride=w*channels; raw=zlib.decompress(payload); out=[]; last=bytes(stride)
 for y in range(h):
  t=raw[y*(stride+1)]; row=bytearray(raw[y*(stride+1)+1:(y+1)*(stride+1)])
  for i in range(stride):
   a=row[i-channels] if i>=channels else 0; b=last[i]; c=last[i-channels] if i>=channels else 0
   if t==1: v=a
   elif t==2: v=b
   elif t==3: v=(a+b)//2
   elif t==4:
    p=a+b-c; d=[abs(p-a),abs(p-b),abs(p-c)]; v=[a,b,c][d.index(min(d))]
   else:v=0
   row[i]=(row[i]+v)&255
  out.extend(row); last=row
 return w,h,channels,out
for file in sorted(root.glob('*.png')):
 w,h,c,data=pixels(file); chromatic=sum(max(data[i:i+3])-min(data[i:i+3])>4 and (c==3 or data[i+3]>128) for i in range(0,len(data),c))
 assert chromatic==0,(file,chromatic)
 print(file.name,w,h,'grayscale verified')
