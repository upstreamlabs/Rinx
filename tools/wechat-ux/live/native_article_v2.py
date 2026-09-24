#!/usr/bin/env python3
"""Real native article journeys on disposable Palpo accounts; never a visual score.

Seeds one AI-generated illustration into the private asset library. The system
file picker is not driven by the Makepad bridge; image preprocessing is tested
separately. All editing, selecting, confirming and reading use native input.
"""
import json, os, time, uuid, shutil
from pathlib import Path
from urllib.parse import quote
from native_probe import NativeApp
from seed import checked


def main():
    os.environ.update(MAKEPAD_HIDE_WINDOWS='1', MAKEPAD_NO_FOCUS='1')
    os.environ.pop('MAKEPAD_FOCUS', None)
    fixture_path=Path('lab/wechat-ux/evidence/live/fixture.json')
    fixture=json.loads(fixture_path.read_text())
    assert all(u['user_id'].startswith('@robrix_ux_') for u in fixture['users'].values())
    root=fixture_path.parent/'article-editor-v2'/uuid.uuid4().hex
    root.mkdir(parents=True, mode=0o700)
    report={'passed':False,'checks':[],'runs':[],'visual_acceptance':False,'system_file_picker_tested':False}
    apps=[]
    def mark(name): report['checks'].append(name); print('PASS '+name,flush=True)
    def prepare(name,alias,lang='zh-CN'):
        path=root/name;path.mkdir(mode=0o700)
        data=json.loads(json.dumps(fixture));data['users']['alex']=fixture['users'][alias]
        (path/'fixture.json').write_text(json.dumps(data));os.chmod(path/'fixture.json',0o600)
        (path/'profile').mkdir();(path/'profile/ui-language.json').write_text(json.dumps(lang));return path
    def start(path,port,size=(430,820),lang='zh-CN'):
        app=NativeApp(path,port,size=size);apps.append(app);app.start();report['runs'].append(str(app.output))
        app.wait_text('全部聊天' if lang=='zh-CN' else 'All Chats',timeout=90);return app
    def tap(app,widget):
        for _ in range(6):
            if any(w['i']==widget for w in app.snap()): return app.click_id(widget)
            app.request('/m',k='scroll',x=210,y=440,dy=360,wait=1);time.sleep(.25)
        raise AssertionError('Control is not reachable by scrolling: '+widget)
    def fill(app,widget,text):
        app.click_id(widget);app.request('/k',c='A',cmd=1,wait=1);app.request('/k',c='Backspace',wait=1)
        if text: app.request('/t',t=text,wait=1)
    def capture(app,name):
        app.capture(name);(app.output/(name+'-widgets.json')).write_text(json.dumps(app.request('/snap'),ensure_ascii=False,indent=2))
    def open_editor(app):
        app.click_id('discover_tab');app.click_id('discover_article');app.wait_text('应用详情');app.click_id('article_continue');app.wait_text('授权使用');app.click_id('article_allow');app.wait_text('图文工作室')
    def library(path):
        file=next((path/'profile').glob('mini-apps/**/library-v2.json'));return file,json.loads(file.read_text())
    def events():
        return checked(fixture['url'],'GET',f"rooms/{quote(fixture['rooms']['emma'],safe='')}/messages?dir=b&limit=100",token=fixture['users']['emma']['access_token'])['chunk']
    def find_event(test,timeout=30):
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            matches=[e for e in events() if test(e)]
            if matches:return matches
            time.sleep(.5)
        raise AssertionError('Recipient did not receive expected article event')
    try:
        sender_path=prepare('sender','alex');sender=start(sender_path,8361)
        open_editor(sender);capture(sender,'01-library');sender.click_id('article_new')
        title='把周末还给山野 '+root.name[:6]
        fill(sender,'article_title',title);fill(sender,'article_author','林间来信')
        sender.click_id('rich');sender.request('/t',t='清晨六点，城市还没有醒来。我们沿着山路出发，把消息提醒留在身后。',wait=1)
        sender.request('/k',c='A',cmd=1,wait=1);sender.click_id('article_bold');sender.click_id('article_save');time.sleep(.5)
        _,lib=library(sender_path);doc=lib['documents'][0];assert doc['blocks'][0]['marks'][0]['bold']
        capture(sender,'02-visual-editor');mark('native_chinese_input_and_selected_text_bold')
        sender.click_id('article_source');fill(sender,'article_markdown','<script>DO_NOT_RENDER</script>');sender.click_id('source_apply');sender.wait_text('源码已保留')
        body='清晨六点，城市还没有醒来。我们沿着山路出发。\n\n## 01 走进山野\n\n> 慢一点，才能看见更多。\n\n**认真感受眼前的风景。**\n\n'+('\n\n'.join('风穿过松林，光落在石阶。第 %d 段，记录属于自己的周末。'%i for i in range(1,8)))+'\n\n全文结束 END OF ARTICLE'
        fill(sender,'article_markdown',body);sender.click_id('source_apply');sender.click_id('article_save');time.sleep(.4)
        file,lib=library(sender_path);asset=json.loads(Path('lab/article-editor-v2/artwork/mountains.json').read_text());lib['assets'][asset['id']]=asset
        (file.parent/'assets').mkdir(exist_ok=True);shutil.copyfile('lab/article-editor-v2/artwork/mountains.png',file.parent/'assets'/asset['id']);os.chmod(file.parent/'assets'/asset['id'],0o600);file.write_text(json.dumps(lib));os.chmod(file,0o600)
        sender.click_id('article_images');sender.wait_text('晨光里的山谷');capture(sender,'03-image-library');sender.click_text('晨光里的山谷');sender.wait_text('编辑文章');capture(sender,'04-inline-image')
        sender.click_id('image_settings');fill(sender,'image_caption','晨光里的山谷');fill(sender,'image_alt','薄雾中的绿色山谷');sender.click_id('image_medium');capture(sender,'05-image-settings');sender.click_id('image_done')
        sender.click_id('article_theme');sender.wait_text('暖纸色');sender.click_text('暖纸色');capture(sender,'06-theme');sender.click_id('theme_done')
        sender.click_id('article_cover');sender.click_id('cover_pick');sender.click_text('晨光里的山谷');fill(sender,'cover_summary','在山路与晨光之间，找回生活的节奏。');capture(sender,'07-cover');sender.click_id('article_done');sender.click_id('article_save');time.sleep(.5)
        _,lib=library(sender_path);doc=lib['documents'][0];assert doc['theme']=='paper' and doc['cover']['asset']==asset['id'];assert any(b['kind']=='image' and b['width']==75 for b in doc['blocks'])
        mark('native_image_selection_settings_theme_cover_and_persistence')
        sender.click_id('article_preview');capture(sender,'08-full-preview');sender.request('/m',k='scroll',x=210,y=450,dy=2400,wait=1);sender.wait_text('END OF ARTICLE',pixels=True);capture(sender,'08-full-preview-end');mark('full_review_reaches_final_paragraph');sender.click_id('preview_check');capture(sender,'09-review');tap(sender,'review_continue');fill(sender,'article_chat_search','Emma');sender.click_text('Emma Wilson');capture(sender,'10-publish-confirmation');tap(sender,'article_confirm');sender.wait_text('已发布',timeout=60);capture(sender,'11-publication')
        received=find_event(lambda e:e.get('content',{}).get('org.octosense.article',{}).get('document',{}).get('title')==title)
        assert len(received)==1;original=received[0];article=original['content']['org.octosense.article'];assert article['document']['theme']=='paper';assert article['assets'][asset['id']]['source']['url'].startswith('mxc://')
        mark('matrix_publish_contains_portable_article_and_uploaded_image')
        recipient_path=prepare('recipient','emma');recipient=start(recipient_path,8362)
        recipient.click_text('Alex Chen');recipient.wait_text(title,pixels=True,timeout=40);capture(recipient,'12-received-article-card');recipient.click_text(title);recipient.wait_text('阅读全文');recipient.wait_text('林间来信');time.sleep(2);capture(recipient,'13-recipient-native-reader')
        assert not any(w['i']=='article_allow' for w in recipient.snap());mark('recipient_opens_native_article_reader')
        tap(sender,'publication_edit');fill(sender,'article_title',title+' · 修订');sender.click_id('article_preview');sender.click_id('preview_check');tap(sender,'review_continue');capture(sender,'14-update-confirmation');tap(sender,'article_confirm');sender.wait_text('已发布',timeout=60)
        updates=find_event(lambda e:e.get('content',{}).get('m.relates_to',{}).get('event_id')==original['event_id'] and e.get('content',{}).get('m.new_content',{}).get('org.octosense.article',{}).get('version')==2);assert len(updates)==1
        recipient.click_id('article_close');recipient.wait_text(root.name[:6],pixels=True,timeout=40);recipient.click_text(root.name[:6]);recipient.wait_text(title+' · 修订');capture(recipient,'15-recipient-updated-reader');mark('update_keeps_original_matrix_identity_and_reader_gets_latest_revision')
        tap(sender,'publication_withdraw');capture(sender,'16-withdraw-confirmation');tap(sender,'withdraw_confirm');sender.wait_text('已撤回',timeout=60)
        for event in [original,updates[0]]:
            response=checked(fixture['url'],'GET',f"rooms/{quote(fixture['rooms']['emma'],safe='')}/event/{quote(event['event_id'],safe='')}",token=fixture['users']['emma']['access_token']);assert not response.get('content',{}).get('org.octosense.article')
            assert not response.get('content',{}).get('m.new_content',{}).get('org.octosense.article')
        _,lib=library(sender_path);assert lib['publications'][0]['withdrawn'] and lib['documents'];mark('withdraw_redacts_original_and_revision_and_preserves_local_draft')
        sender.click_id('article_back');sender.click_id('article_share');fill(sender,'article_chat_search','Emma');sender.click_text('Emma Wilson');tap(sender,'article_confirm');sender.wait_text('小应用已发送',timeout=45)
        recipient.click_id('article_close');recipient.wait_text('文章编辑器',pixels=True,timeout=40);recipient.click_text('文章编辑器');recipient.click_id('article_continue');recipient.wait_text(fixture['users']['emma']['user_id']);capture(recipient,'17-recipient-own-consent');recipient.click_id('article_allow');recipient.click_id('article_new');assert next(w for w in recipient.snap() if w['i']=='article_title').get('val','')=='';mark('shared_editor_requires_recipient_consent_and_does_not_share_sender_drafts')
        sender.stop();apps.remove(sender);(sender_path/'profile/ui-language.json').write_text('"en"')
        english=start(sender_path,8361,size=(1440,960),lang='en');english.click_text('Emma Wilson');english.click_id('open_popup_menu_button');english.click_id('article_editor_button');english.click_id('article_continue');english.click_id('article_allow');english.click_text(root.name[:6]);english.wait_text('Edit article');capture(english,'18-desktop-editor-en');assert any(w['i']=='inspector_theme' for w in english.snap());mark('english_desktop_three_column_editor')
        report['passed']=True
    finally:
        if not report['passed']:
            for app in apps:
                if app.process and app.process.poll() is None:
                    try:capture(app,'failure')
                    except Exception:pass
        for app in apps:app.stop()
        (root/'result.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
        print(json.dumps({'passed':report['passed'],'evidence':str(root),'checks':report['checks']}),flush=True)

if __name__=='__main__':main()
