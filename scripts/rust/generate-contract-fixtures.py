"""Generate synthetic golden cases using the preserved Python implementation."""
import json, random, sys
from pathlib import Path
sys.path.insert(0,str(Path('tests/oracle').resolve()))
from bridge import normalize_question,question_key,format_answer
rng=random.Random(20261004)
raws=[{'answers':['A'],'confident':True,'explanation':'依据','sources':[]},{'answers':['甲','乙','甲'],'confident':True,'explanation':'依据','sources':[]},{'answers':[],'confident':False,'explanation':'缺少图','sources':[]},{'answers':['false'],'confident':True,'explanation':'依据','sources':[]},{'answers':[3],'confident':True,'explanation':'bad'}, {'answers':[''],'confident':True,'explanation':''}]
questions=[[],{}, {'title':'  '},{'title':'a','options':None},{'title':'a','type':False},{'title':'x'*16001}]
for i in range(400):
 questions.append({'title':rng.choice(['选择题',' 题\t 目\u001c甲 ','填空🙂','简答\n换行'])+str(i%25),'options':rng.choice(['','A. 甲\nB. 乙','Ａ．甲\r\nＢ．乙','A. B\nB. A','A.甲\u2028B.乙',['A. 甲','B. 乙'],'${options}','甲\u00a0\t乙\n丙']),'type':rng.choice(['single','multiple','completion','judgement','','${type}','undefined'])})
fixtures=[]
for data in questions:
 item={'input':data,'raw':rng.choice(raws)}
 try:
  q=normalize_question(data);item['normalized']=q;item['key']=question_key(q)
  try:item['answer']=format_answer(q,item['raw'])
  except Exception as e:item['answer_error']=str(e)
 except Exception as e:item['error']=str(e)
 fixtures.append(item)
Path('crates/ocs-core/tests/contracts.json').write_text(json.dumps(fixtures,ensure_ascii=False,indent=2))
print(f'{len(fixtures)} synthetic differential fixtures')
