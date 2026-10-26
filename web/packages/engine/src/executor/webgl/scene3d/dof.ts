export const scene3dDofFragment = `#version 300 es
precision highp float;precision highp int;out vec4 result;uniform highp sampler2D source;uniform highp sampler2D rawSource;uniform highp sampler2D depths;
uniform vec2 size;uniform vec4 visibleBounds;uniform vec4 nearBounds;uniform float focus;uniform float maxRadius;uniform bool hasDof;

uniform uint precisionBarrier;
float f32(float v){return uintBitsToFloat(floatBitsToUint(v)^precisionBarrier);}
vec3 f32v(vec3 v){return uintBitsToFloat(floatBitsToUint(v)^uvec3(precisionBarrier));}
vec4 f32c(vec4 v){return uintBitsToFloat(floatBitsToUint(v)^uvec4(precisionBarrier));}
vec3 toLinear(vec3 c){return mix(c/12.92,pow((c+.055)/1.055,vec3(2.4)),step(vec3(.04045),c));}
vec3 toSRGB(vec3 c){c=max(c,vec3(0));return mix(c*12.92,1.055*pow(c,vec3(1./2.4))-.055,step(vec3(.0031308),c));}
vec4 decode(vec4 c){return c.a>0.?vec4(toLinear(clamp(c.rgb/c.a,0.,1.))*c.a,c.a):vec4(0);}
vec4 encode(vec4 c){float a=clamp(c.a,0.,1.);return a>0.?vec4(toSRGB(clamp(c.rgb/a,0.,1.))*a,a):vec4(0);}
const vec2 dirs[8]=vec2[8](vec2(1,0),vec2(.70710677,.70710677),vec2(0,1),vec2(-.70710677,.70710677),vec2(-1,0),vec2(-.70710677,-.70710677),vec2(0,-1),vec2(.70710677,-.70710677));
const float rings[4]=float[4](.125,.25,.5,1.);const float weights[4]=float[4](4.,2.,1.,.5);
bool inside(vec2 p,vec4 b){return p.x>=b.x&&p.y>=b.y&&p.x<b.z&&p.y<b.w;}
ivec2 physical(ivec2 p){return p;}
float zAt(ivec2 p){uvec4 c=uvec4(floor(texelFetch(depths,physical(p),0)*255.0+.5));return uintBitsToFloat(c.x|(c.y<<8u)|(c.z<<16u)|(c.w<<24u));}
float coc(float z){return min(f32(maxRadius*abs(f32(1.-f32(focus/z)))),maxRadius);}
ivec2 rounded(vec2 p){return ivec2(f32(p.x+sign(p.x)*.4999999701976776),f32(p.y+sign(p.y)*.4999999701976776));}
void main(){
 ivec2 p=ivec2(int(gl_FragCoord.x),int(size.y)-1-int(gl_FragCoord.y));vec4 raw=texelFetch(rawSource,physical(p),0);if(!hasDof){result=raw;return;}
 float z=zAt(p),radius=coc(z),tolerance=max(f32(z*.005),.005);vec4 original=texelFetch(source,physical(p),0),color=original;
 if(radius>=.75){vec4 sum=original;float count=1.;for(int r=0;r<4;r++)for(int d=0;d<8;d++){
  ivec2 q=rounded(vec2(f32(float(p.x)+f32(f32(dirs[d].x*radius)*rings[r])),f32(float(p.y)+f32(f32(dirs[d].y*radius)*rings[r]))));if(!inside(vec2(q),vec4(0,0,size)))continue;float qz=zAt(q);
  if(z<focus&&qz>f32(z+tolerance))continue;if(qz<f32(z-tolerance)&&(qz<=focus||length(vec2(q-p))>coc(qz)))continue;
  sum+=texelFetch(source,physical(q),0);count+=1.;}color=sum/count;}
 if(inside(vec2(p),nearBounds)){vec4 sum=vec4(0);for(int r=0;r<4;r++)for(int d=0;d<8;d++){
  float radiusAt=r==0?1.:r==3?maxRadius:f32(f32(maxRadius*float(r))/3.);ivec2 q=rounded(vec2(f32(float(p.x)+f32(dirs[d].x*radiusAt)),f32(float(p.y)+f32(dirs[d].y*radiusAt))));if(!inside(vec2(q),vec4(0,0,size)))continue;
  float qz=zAt(q);if(qz>=focus||qz>=f32(z-tolerance)||length(vec2(q-p))>coc(qz))continue;sum+=texelFetch(source,physical(q),0)*weights[r];}
  vec4 near=sum/60.;if(near.a>0.)color=near+color*(1.-near.a);}
 result=radius<.75&&all(equal(color,original))?raw:encode(color);
}`;

export const scene3dDecodeFragment = `#version 300 es
precision highp float;precision highp int;out vec4 result;uniform highp sampler2D source;

uniform uint precisionBarrier;
float f32(float v){return uintBitsToFloat(floatBitsToUint(v)^precisionBarrier);}
vec3 f32v(vec3 v){return uintBitsToFloat(floatBitsToUint(v)^uvec3(precisionBarrier));}
vec4 f32c(vec4 v){return uintBitsToFloat(floatBitsToUint(v)^uvec4(precisionBarrier));}
vec3 toLinear(vec3 c){return mix(c/12.92,pow((c+.055)/1.055,vec3(2.4)),step(vec3(.04045),c));}
vec3 toSRGB(vec3 c){c=max(c,vec3(0));return mix(c*12.92,1.055*pow(c,vec3(1./2.4))-.055,step(vec3(.0031308),c));}
vec4 decode(vec4 c){return c.a>0.?vec4(toLinear(clamp(c.rgb/c.a,0.,1.))*c.a,c.a):vec4(0);}
vec4 encode(vec4 c){float a=clamp(c.a,0.,1.);return a>0.?vec4(toSRGB(clamp(c.rgb/a,0.,1.))*a,a):vec4(0);}
void main(){result=decode(texelFetch(source,ivec2(gl_FragCoord.xy),0));}`;
