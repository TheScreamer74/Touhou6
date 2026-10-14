/* Optional read-only original ANM observations, separate from gameplay pools. */
#ifndef TH06_REFCAP_VM_TRACE_H
#define TH06_REFCAP_VM_TRACE_H
static FILE *g_vm_trace;
static unsigned g_vm_frames[256][2], g_nvm_frames;
static int refcap_vm_config(const char *line)
{
    char path[64];unsigned stage,frame;
    if(sscanf(line,"vmfile %63s",path)==1){g_vm_trace=fopen(path,"w");return 1;}
    if(sscanf(line,"vmframe %u %u",&stage,&frame)==2){
        if(g_nvm_frames<256){g_vm_frames[g_nvm_frames][0]=stage;g_vm_frames[g_nvm_frames++][1]=frame;}
        return 1;
    }
    return 0;
}
#define VM_F(p,o) (*(const float*)((p)+(o)))
static void refcap_vm_log(unsigned stage,unsigned frame)
{
    if(!g_vm_trace)return;
    int wanted=0;
    for(unsigned i=0;i<g_nvm_frames;i++)if(g_vm_frames[i][0]==stage&&g_vm_frames[i][1]==frame){
        wanted=1;g_vm_frames[i][0]=0;
    }
    if(!wanted)return;
    const unsigned char *player=(const unsigned char*)0x6ca628;
    fprintf(g_vm_trace,"{\"stage\":%u,\"frame\":%u,\"bomb\":%u,\"bomb_timer\":%d,\"opts\":%u,\"sprites\":[",stage,frame,
        *(const unsigned*)(player+0x75c8),*(const int*)(player+0x75d8),*(const unsigned*)(0x6c6d18+0x114+52));
    for(unsigned i=0;i<32;i++){
        const unsigned char *vm=player+0x76e4+i*0x110;
        fprintf(g_vm_trace,"%s{\"slot\":%u,\"script\":%d,\"sprite\":%d,\"flags\":%u,\"color\":%u,\"x\":%.9g,\"y\":%.9g,\"z\":%.9g,\"offset_x\":%.9g,\"offset_y\":%.9g,\"scale_x\":%.9g,\"scale_y\":%.9g,\"rotation\":%.9g,\"time\":%d}",
            i?",":"",i,*(const short*)(vm+0xb4),*(const short*)(vm+0xb0),*(const unsigned short*)(vm+0x80),*(const unsigned*)(vm+0x7c),
            VM_F(vm,0x90),VM_F(vm,0x94),VM_F(vm,0x98),VM_F(vm,0xe4),VM_F(vm,0xe8),VM_F(vm,0x1c),VM_F(vm,0x18),VM_F(vm,0x8),*(const int*)(vm+0x38));
    }
    fprintf(g_vm_trace,"],\"player_target\":[%.9g,%.9g,%.9g],\"enemies\":[",VM_F(player,0xa1c),VM_F(player,0xa20),VM_F(player,0xa24));
    const char *comma="";
    for(unsigned i=0;i<256;i++){
        const unsigned char *e=(const unsigned char*)0x4b79c8+8+0xec8+i*0xec8;
        if(!(e[0xe50]&0x80))continue;
        fprintf(g_vm_trace,"%s{\"slot\":%u,\"flags\":%u,\"script\":%d,\"sprite\":%d}",comma,i,*(const unsigned*)(e+0xe50),*(const short*)(e+0xb4),*(const short*)(e+0xb0));
        comma=",";
    }
    fputs("]}\n",g_vm_trace);fflush(g_vm_trace);
}
#undef VM_F
#endif
