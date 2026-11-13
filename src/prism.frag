#version 150 core
uniform vec2 resolution;
uniform vec2 origin;
uniform float rotation, hue, opacity;
uniform vec3 scene;
out vec4 fragment_color;
#define PI 3.14159265359
vec3 palette(float t,float seed){return .52+.48*cos(6.28318*(vec3(0.,.33,.67)+t)+seed);}
float hash(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);}
vec3 pattern(vec2 uv,vec3 scene){
 float mode=scene.x,seed=scene.y,n=scene.z;
 float r=length(uv),a=atan(uv.y,uv.x)-rotation+seed;
 float wedge=2.*PI/n;
 a=abs(mod(a+wedge*.5,wedge)-wedge*.5);
 vec2 p=vec2(cos(a),sin(a))*r;
 float t=seed*.16;float h=hue+seed*.13;
 vec3 col=vec3(0.);
 if(mode<.5){
  float petals=cos(a*n)*.5+.5;
  float wave=sin(r*24.-t*2.+sin(a*n)*2.8+sin(r*9.+t));
  float veins=abs(sin(r*36.+petals*5.+t));
  float bead=pow(max(0.,sin(r*19.-t+petals*3.)),10.);
  col=palette(r*.48+wave*.16+h,seed)*(.16+.85*pow(abs(wave),2.));
  col+=palette(r*.3+h+.3,seed)*pow(veins,24.)*.9;
  col+=vec3(.6,.85,1.)*bead*pow(petals,7.)*.7;
  col*=.45+.55*smoothstep(.02,.2,abs(wave));
 }else if(mode<1.5|| (mode>2.5&&mode<3.5)){
  vec2 q=p*(mode<1.5?11.:18.);
  q+=vec2(sin(r*4.+t)*.55,t*.4);
  q.x+=.65*sin(q.y*.55+seed);
  vec2 cell=floor(q);vec2 f=fract(q);
  float diagonal=step(f.x,f.y);float rnd=hash(cell+seed+diagonal*13.);
  float edge=min(min(f.x,1.-f.x),min(f.y,1.-f.y));edge=min(edge,abs(f.x-f.y)*.707);
  col=palette(rnd*.9+h,seed)*(.55+rnd*.55);
  if(mode<1.5)col*=smoothstep(.015,.055,edge);else col*=.7+.3*smoothstep(0.,.08,edge);
  col+=pow(max(0.,1.-length(f-vec2(.25))*.9),16.)*.2;
 }else if(mode<2.5){
  float petal=pow(abs(cos(a*n*.5)),3.);
  float bands=sin(r*22.-petal*5.-t);
  float gem=pow(max(0.,1.-abs(bands)),5.);
  col=palette(r*.8+petal*.25+h,seed)*gem*1.4;
  col+=vec3(.65,.9,1.)*pow(gem,12.)*.65;
  col+=palette(r+h+.4,seed)*pow(abs(sin(r*33.+petal*6.+t)),40.)*.65;
  col*=(1.-smoothstep(.65,1.32,r));
 }else if(mode<4.5){
  float warp=sin(r*12.-t)*.13;
  float lace=abs(sin(a*n*1.5+sin(r*18.+t)*2.));
  float ring=abs(sin(r*28.+sin(a*n)*2.+t));
  col=palette(r*.6+h,seed)*(.08+.9*pow(lace,20.)+.75*pow(ring,26.));
  col+=palette(r+h+.45,seed)*pow(abs(sin(r*16.+warp*12.+cos(a*n)*4.)),30.);
 }else if(mode<5.5){
  // Interfering waves produce broad, pearlescent ribbons.
  float flow=sin(p.x*15.+sin(p.y*12.+t)*2.)+cos(r*19.-t*1.3);
  float ripple=sin(flow*2.4+cos(a*n)*2.+t);
  col=palette(flow*.12+r*.25+h,seed)*(.35+.65*pow(abs(ripple),3.));
  col+=vec3(.7,.85,1.)*pow(abs(ripple),32.)*.7;
 }else if(mode<6.5){
  // Logarithmic rings and tiny lights recede into a star tunnel.
  float depth=-log(r+.055);
  float web=abs(sin(depth*14.+sin(a*n)*2.-t*2.));
  float spoke=abs(sin(a*n*2.+depth*3.+t));
  col=palette(depth*.22+h,seed)*(.8*pow(web,35.)+.65*pow(spoke,45.));
  float stars=pow(max(0.,sin(depth*31.-t)*cos(a*n*3.)),24.);
  col+=vec3(.8,.9,1.)*stars*1.8;
 }else if(mode<7.5){
  // Rounded nested teardrops with luminous rims.
  float lobe=cos(a*n);
  float fold=sin(r*17.-lobe*3.5+sin(r*5.-t)*2.-t);
  float inner=sin(r*29.+lobe*5.+t*.7);
  col=palette(r*.65+fold*.15+h,seed)*(.15+.85*pow(abs(fold),.65));
  col*=.3+.7*smoothstep(.03,.3,abs(inner));
  col+=palette(h+r+.35,seed)*pow(abs(inner),48.)*.9;
 }else if(mode<8.5){
  // Mirrored right-angle paths form a moving geometric maze.
  vec2 q=p*16.+vec2(t*.35,sin(t*.4));
  vec2 cell=floor(q);vec2 f=fract(q)-.5;
  float turn=step(.5,hash(cell+seed));
  float path=min(abs(f.x),abs(f.y));
  if(turn>.5)path=abs(max(abs(f.x),abs(f.y))-.32);
  float line=1.-smoothstep(.035,.09,path);
  col=palette(hash(cell+seed)*.3+r*.4+h,seed)*(line*.95+.035);
  col+=vec3(.65,.85,1.)*(1.-smoothstep(.0,.018,path))*.45;
 }else{
  // Fine golden threads interlace around a radiant flower.
  float petal=cos(a*n);
  float thread=abs(sin(r*48.+petal*5.+sin(r*8.-t)*2.));
  float braid=abs(sin(a*n*3.+r*12.+t));
  vec3 gold=palette(h+.1,seed);
  col=mix(gold,palette(r*.3+h,seed),.3+.2*sin(t*.3));
  col*=.08+.85*pow(thread,28.)+.6*pow(braid,36.);
  col+=vec3(1.,.9,.6)*pow(thread*braid,20.)*.8;
 }

 float heart=exp(-r*35.);col+=palette(h+.2,seed)*heart*.9;
 col*=1.-.22*smoothstep(.65,1.6,r);
 return col;
}

void main() {
    vec3 color = vec3(0.);
    // Four subpixel samples reduce aliasing in fine filigree and mirrored seams.
    for (int y = 0; y < 2; ++y) {
        for (int x = 0; x < 2; ++x) {
            vec2 offset = (vec2(x, y) + .5) * .5 - .5;
            vec2 uv = (gl_FragCoord.xy - origin + offset - .5 * resolution)
                / min(resolution.x, resolution.y) * 2.15;
            uv.y = -uv.y;
            float low = floor(scene.z * .5) * 2.;
            float weight = (scene.z - low) * .5;
            vec3 c = pattern(uv, vec3(scene.xy, low));
            if (weight > .001) {
                c = mix(c, pattern(uv, vec3(scene.xy, min(28., low + 2.))), weight);
            }
            c = pow(1. - exp(-c * 1.35), vec3(.88));
            color += c * .25;
        }
    }
    fragment_color = vec4(color * opacity, opacity);
}
