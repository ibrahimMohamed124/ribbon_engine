//! CPU shelf packing primitives for the glyph mask atlas.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct PackedGlyph{pub x:u32,pub y:u32,pub w:u32,pub h:u32}
#[derive(Default)]pub struct GlyphAtlas{width:u32,height:u32,x:u32,y:u32,row:u32}
impl GlyphAtlas{pub fn new()->Self{Self{width:1024,height:1024,..Default::default()}}pub fn pack(&mut self,w:u32,h:u32)->Option<PackedGlyph>{if w>self.width||h>self.height{return None}if self.x+w>self.width{self.x=0;self.y+=self.row;self.row=0}if self.y+h>self.height{return None}let p=PackedGlyph{x:self.x,y:self.y,w,h};self.x+=w;self.row=self.row.max(h);Some(p)}}
