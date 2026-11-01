const s=[{id:"first_name",label:"First name",code:'{{ contact.first_name or "there" }}'},{id:"last_name",label:"Last name",code:"{{ contact.last_name }}"},{id:"full_name",label:"Full name",code:'{{ contact.full_name or "there" }}'},{id:"email",label:"Email address",code:"{{ contact.email }}"}],c={label:"Unsubscribe link",code:'<a href="{{ unsubscribe_url }}" style="color:inherit;">Unsubscribe</a>'},o=/^[a-z][a-z0-9_]{0,39}$/,p=e=>o.test(e),h=e=>`{{ contact.attributes.${e} }}`,t="font-family:Arial,Helvetica,sans-serif;",d=[{id:"heading",label:"Heading",code:`<h1 style="margin:0 0 16px;${t}font-size:26px;line-height:1.25;color:#111827;">A clear headline</h1>`},{id:"paragraph",label:"Paragraph",code:`<p style="margin:0 0 16px;${t}font-size:16px;line-height:1.5;color:#374151;">Say something worth reading.</p>`},{id:"button",label:"Button",code:`<table role="presentation" cellpadding="0" cellspacing="0" border="0" style="margin:8px 0 24px;"><tr><td style="border-radius:6px;background:#2f5bea;"><a href="https://example.com" style="display:inline-block;padding:12px 22px;${t}font-size:16px;font-weight:bold;color:#ffffff;text-decoration:none;border-radius:6px;">Read more</a></td></tr></table>`},{id:"image",label:"Picture",code:'<img src="https://" alt="Describe the picture" width="560" style="display:block;width:100%;max-width:560px;height:auto;border:0;margin:0 0 16px;">'},{id:"divider",label:"Divider",code:'<hr style="border:0;border-top:1px solid #e5e7eb;margin:24px 0;">'}],b=`<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title></title></head>
<body style="margin:0;padding:0;background:#f3f4f6;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="background:#f3f4f6;"><tr><td align="center" style="padding:24px 12px;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="max-width:600px;background:#ffffff;border-radius:8px;"><tr><td style="padding:32px;${t}font-size:16px;line-height:1.5;color:#374151;">
<h1 style="margin:0 0 16px;${t}font-size:26px;line-height:1.25;color:#111827;">Hello {{ contact.first_name or "there" }},</h1>
<p style="margin:0 0 16px;">Tell people what is new, in a sentence or two.</p>
${d.find(e=>e.id==="button").code}
<p style="margin:0;">Thanks,<br>The team</p>
</td></tr></table>
</td></tr></table>
</body>
</html>
`;function g(e,l,n,i){const a=Math.max(0,Math.min(l,e.length)),r=Math.max(a,Math.min(n,e.length));return{text:e.slice(0,a)+i+e.slice(r),caret:a+i.length}}export{d as B,s as M,b as S,c as U,h as a,g as i,p as v};
