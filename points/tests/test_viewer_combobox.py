"""件数を右寄せするコンボボックスの選択・キー操作・表示更新を確認する。"""
from pathlib import Path
import shutil
import subprocess
import unittest

class CountComboboxTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('node'), 'Node.js が必要です')
    def test_selection_keyboard_and_programmatic_reset(self):
        page=(Path(__file__).resolve().parents[1]/'viewer.html').read_text(encoding='utf-8')
        source='function splitOptionCount('+page.split('function splitOptionCount(',1)[1].split('function refreshProvenanceFilters()',1)[0]
        checks=r"""
const assert=require('node:assert/strict');
class Element{
 constructor(){this.children=[];this.style={};this.attrs={};this.events={};this.hidden=false;this.classList={add(){},toggle(){}};}
 append(...nodes){this.children.push(...nodes);}
 replaceChildren(...nodes){this.children=nodes;}
 setAttribute(k,v){this.attrs[k]=v;} removeAttribute(k){delete this.attrs[k];}
 addEventListener(k,f){(this.events[k]??=[]).push(f);}
 dispatchEvent(event){for(const f of this.events[event.type]??[])f(event);}
 scrollIntoView(){} focus(){} getBoundingClientRect(){return{right:1200};}
 contains(node){return this===node||this.children.some(child=>child.contains?.(node));}
}
const document=new Element();document.createElement=()=>new Element();const window={innerWidth:1000};
class MutationObserver{constructor(callback){this.callback=callback;}observe(){}}
const select=new Element(),parent=new Element();parent.firstChild={textContent:'出典'};select.parentElement=parent;select.id='source-filter';
select.options=[{value:'all',textContent:'すべて (20 件)',selected:true},{value:'vector',textContent:'[vector] ベクトルタイル (15 件)',selected:false}];
Object.defineProperty(select,'selectedIndex',{get(){return this.options.findIndex(o=>o.selected);},set(index){this.options.forEach((o,i)=>o.selected=i===index);}});
Object.defineProperty(select,'selectedOptions',{get(){return this.options.filter(o=>o.selected);}});
let changed=0;select.addEventListener('change',()=>changed++);
const control=installCountCombobox(select),[button,list]=parent.children;
assert.equal(button.children[0].textContent,'すべて');assert.equal(button.children[1].textContent,'(20 件)');
assert.equal(button.children[1].className,'count-combobox-count');
button.dispatchEvent({type:'click'});assert.equal(list.hidden,false);assert.equal(list.style.left,'-208px');
list.children[1].dispatchEvent({type:'click'});assert.equal(select.selectedIndex,1);assert.equal(changed,1);assert.equal(list.hidden,false);
assert.equal(button.children[0].textContent,'[vector] ベクトルタイル');assert.equal(button.children[1].textContent,'(15 件)');
select.selectedIndex=0;control.sync();assert.equal(button.children[0].textContent,'すべて');button.dispatchEvent({type:'click'});
const key=key=>button.dispatchEvent({type:'keydown',key,preventDefault(){}});
key('ArrowDown');key('ArrowDown');key('Enter');assert.equal(select.selectedIndex,1);assert.equal(list.hidden,false);
key('Escape');key('ArrowDown');key('Escape');assert.equal(button.attrs['aria-expanded'],'false');
select.options.push({value:'natural',textContent:'[natural] 自然地名 (5 件)',selected:false});control.sync();list.children[2].dispatchEvent({type:'click'});assert.equal(select.selectedOptions.length,2);assert.equal(button.children[0].textContent,'2 項目を選択');assert.equal(list.children[2].children[0].checked,true);list.children[1].dispatchEvent({type:'click'});assert.equal(select.selectedOptions.length,1);assert.equal(select.selectedOptions[0].value,'natural');list.children[0].dispatchEvent({type:'click'});assert.equal(select.selectedOptions[0].value,'all');
button.dispatchEvent({type:'click'});document.dispatchEvent({type:'pointerdown',target:new Element()});assert.equal(list.hidden,true);
"""
        result=subprocess.run([shutil.which('node'),'-e',source+checks],capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr.decode('utf-8',errors='replace'))
