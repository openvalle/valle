export const composition = { width: 64, height: 64, fps: 30, duration: 5 };
export const controls = defineControls({assets:{model:asset({kind:"model3d",required:true})}});
export default function Model(ctx) {return <Scene style={{width:64,height:64,backgroundColor:"#000000"}}>
<Scene3D key="test" pbr={{toneMapping:"aces",exposure:1.1}} camera={{position:[0,0,3],target:[0,0,0],fov:45}} style={{width:64,height:64}}>
<Mesh key="tri" src="asset://model" rotateY={ctx.seconds * 60} material={{type:"pbr",color:"#ffffff"}} />
<HemisphereLight skyColor="#bad6ff" groundColor="#1a1a22" intensity={1.1} />
<DirectionalLight direction={[0,0,1]} intensity={3} /></Scene3D></Scene>;}
