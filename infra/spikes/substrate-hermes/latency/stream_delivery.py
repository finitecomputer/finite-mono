"""Spike SimpleX incremental delivery via stable message IDs and native edits."""
import asyncio,json
from latency_trace import mode,trace

def install(adapter):
    from gateway.platforms.base import SendResult
    original=adapter.send
    async def send(chat_id,content,reply_to=None,metadata=None):
        if not mode().get('streaming') or 'MEDIA:' in content or not content or asyncio.current_task() is adapter._ws_task:
            return await original(chat_id,content,reply_to=reply_to,metadata=metadata)
        target='#'+chat_id[6:] if chat_id.startswith('group:') else '@'+chat_id
        trace('simplex_send')
        response=await adapter._send_command('/_send '+target+' json '+json.dumps([{'msgContent':{'type':'text','text':content}}]),timeout=10)
        if response and response.get('type')=='newChatItems':
            items=response.get('chatItems',[])
            if items:
                mid=items[0].get('chatItem',{}).get('meta',{}).get('itemId')
                if mid is not None:return SendResult(success=True,message_id=str(mid))
        return SendResult(success=False,error='SimpleX send was not acknowledged')
    async def edit(chat_id,message_id,content,*,finalize=False):
        target='#'+chat_id[6:] if chat_id.startswith('group:') else '@'+chat_id
        payload={'msgContent':{'type':'text','text':content},'mentions':{}}
        trace('simplex_edit',final=finalize)
        response=await adapter._send_command('/_update item '+target+' '+str(int(message_id))+' json '+json.dumps(payload),timeout=10)
        success=bool(response and response.get('type')=='chatItemUpdated')
        return SendResult(success=success,message_id=str(message_id),error=None if success else 'SimpleX edit was not acknowledged')
    adapter.send=send;adapter.edit_message=edit
