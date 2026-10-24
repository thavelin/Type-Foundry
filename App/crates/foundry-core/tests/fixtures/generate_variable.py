"""Rebuild the tiny original variable-font fixture. Requires fontTools only for regeneration.
No third-party outlines. Rust tests read the committed result without Python.
"""
from pathlib import Path
from tempfile import TemporaryDirectory
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.designspaceLib import DesignSpaceDocument, AxisDescriptor, SourceDescriptor, InstanceDescriptor
from fontTools.varLib import build
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString

DEST=Path(__file__).with_name('OriginalVariable.ttf')
def master(path, stem, weight):
    chars='HnoxABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 '
    order=['.notdef']+list(dict.fromkeys(chars))
    fb=FontBuilder(1000,isTTF=True);fb.setupGlyphOrder(order);fb.setupCharacterMap({ord(c):c for c in order[1:]})
    glyphs={}
    for name in order:
        p=TTGlyphPen(None)
        if name!=' ':
            h=700 if name.isupper() else 500
            # A simple original block stem with compatible point topology.
            for pt in [(50,0),(50+stem,0),(50+stem,h),(50,h)]:
                if pt==(50,0):p.moveTo(pt)
                else:p.lineTo(pt)
            p.closePath()
        glyphs[name]=p.glyph()
    fb.setupGlyf(glyphs);fb.setupHorizontalMetrics({g:(600 if g.isupper() else 500,50) for g in order})
    fb.setupHorizontalHeader(ascent=800,descent=-200)
    fb.setupNameTable({'familyName':'Original Analyzer Variable','styleName':str(weight),'uniqueFontIdentifier':'Original-Analyzer-Variable-'+str(weight),'fullName':'Original Analyzer Variable '+str(weight),'psName':'OriginalAnalyzerVariable-'+str(weight),'version':'Version 1.000'})
    fb.setupOS2(sTypoAscender=800,sTypoDescender=-200,sxHeight=500,sCapHeight=700,usWeightClass=weight)
    fb.setupPost();fb.setupMaxp();fb.font['head'].created=fb.font['head'].modified=2082844800
    addOpenTypeFeaturesFromString(fb.font,'@Left=[A B C D E F G H I J K L M]; @Right=[N O P Q R S T U V W X Y Z]; feature kern { pos n o -20; pos @Left @Right -40; } kern;')
    fb.save(path)
with TemporaryDirectory() as tmp:
    tmp=Path(tmp);doc=DesignSpaceDocument();axis=AxisDescriptor();axis.name='Weight';axis.tag='wght';axis.minimum=100;axis.default=100;axis.maximum=900;doc.addAxis(axis)
    for stem,weight in [(50,100),(150,900)]:
        path=tmp/f'{weight}.ttf';master(path,stem,weight);src=SourceDescriptor();src.path=str(path);src.name=str(weight);src.location={'Weight':weight};src.copyInfo=src.copyLib=src.copyFeatures=weight==100;doc.addSource(src)
    for name,weight in [('Light',100),('Bold',900)]:
        ins=InstanceDescriptor();ins.name=name;ins.familyName='Original Analyzer Variable';ins.styleName=name;ins.location={'Weight':weight};doc.addInstance(ins)
    d=tmp/'fixture.designspace';doc.write(d);font,_,_=build(d);font.recalcTimestamp=False;font['head'].created=font['head'].modified=2082844800;font.save(DEST)
print(DEST)

from fontTools.ttLib import TTFont, TTCollection
collection=TTCollection();collection.fonts=[TTFont(DEST),TTFont(DEST)];collection.save(DEST.with_name('OriginalCollection.ttc'))
woff=TTFont(DEST);woff.flavor='woff';woff.save(DEST.with_suffix('.woff'))
