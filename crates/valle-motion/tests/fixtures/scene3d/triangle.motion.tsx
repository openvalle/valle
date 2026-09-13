export const controls = defineControls({assets:{model:asset({kind:"model3d",required:true})}});
export default function Model(ctx) {return <Scene style={{width:64,height:64,backgroundColor:"#000000"}}>
<Scene3D key="test" camera={{position:[0,0,3],target:[0,0,0],fov:45}} style={{width:64,height:64}}>
<Mesh key="tri" src="asset://model" rotateY={ctx.seconds * 60} material={{type:"unlit",color:"#efb343"}} />
</Scene3D></Scene>;}
